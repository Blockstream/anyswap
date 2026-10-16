//! The loop: `observe` reads the chain and the server into something
//! verified, `decide` is the pure choice of what to do, and each tick applies
//! it and saves what changed.

use std::cmp::Reverse;

use anyswap_core::api::{CancelRequest, RevealClaimRequest, RevealPreimageRequest};

use super::{
    Driver, MaybeSend, MaybeSync, Registered,
    decide::{Action, Decision, Message, Path, Spend, decide, funds},
    error::DriverError,
    observe::{FUNDS_PER_PASS, Observed, driven, observe},
    record::{FundState, Status, SwapRecord},
    rt, wallet_error,
};
use crate::{
    swap_script::{SwapScript, Terms, Tx},
    utxo::SwapUtxo,
    wallet::SwapWallet,
};

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    /// One pass that refunds to the swap's refund address or the hook's.
    pub(crate) async fn tick(&self, record: &mut SwapRecord) -> Result<(), DriverError> {
        self.pass(record, None).await.map(drop)
    }

    /// One pass that refunds to `destination`, returning the refund's txid.
    pub(crate) async fn refund(
        &self,
        record: &mut SwapRecord,
        destination: &str,
    ) -> Result<Option<String>, DriverError> {
        self.pass(record, Some(destination)).await
    }

    /// One pass: observe, keep what the server proved, decide, apply, refund
    /// what of ours is refundable, then save and announce what changed, an
    /// earlier error it clears included. A
    /// failed action fails the pass only at its end, so a message the server
    /// cannot take never holds back a refund the chain alone allows.
    async fn pass(
        &self,
        record: &mut SwapRecord,
        destination: Option<&str>,
    ) -> Result<Option<String>, DriverError> {
        let terms = driven(record)?;
        let chain = self.chain(terms.chain)?;
        let script = self.core.ctx.script(&terms)?;
        let observed = observe(
            record,
            &terms,
            &script,
            chain,
            &self.core.client,
            &self.config,
        )
        .await?;
        let before = record.clone();

        if let Some(server) = &observed.server {
            record.server_status = Some(server.label.clone());
            if record.server_cancel.is_none() {
                record.server_cancel = server.cancel.clone();
            }
        }
        if let Some(key) = observed.server_privkey() {
            record.endpoint_mut(terms.side).server_privkey = Some(key);
        }
        if let Some(preimage) = observed.preimage() {
            record.preimage = Some(preimage);
        }
        let decision = decide(record, &terms, &observed, &self.config, &chain.config);
        let applied = self.apply(record, &terms, &script, decision, chain).await;
        record.funds = funds(record, &terms, &observed, &chain.config, &record.status);
        if record.status.is_terminal() {
            record.ended_at.get_or_insert(observed.now);
        }
        if *record != before {
            self.core.store.save(record).await?;
            self.emit(record);
        }

        // Announced apart, so a refund that fails cannot hide what the pass
        // already saved.
        let refund = self
            .refund_funds(record, &terms, &script, &observed, chain, destination)
            .await;
        if matches!(refund, Ok(Some(_))) {
            self.core.store.save(record).await?;
            self.emit(record);
        }
        applied?;
        let txid = refund?;
        let cleared = self.core.errors.lock().unwrap().remove(&record.swap_id);
        if cleared.is_some() {
            self.emit(record);
        }
        Ok(txid)
    }

    /// Runs the decision's action and keeps its status either way, corrected
    /// by what the action did: the claim it broadcast, a cancel only once
    /// delivered.
    async fn apply(
        &self,
        record: &mut SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        decision: Decision,
        chain: &Registered,
    ) -> Result<(), DriverError> {
        let Decision { action, mut status } = decision;
        let applied = match action {
            None => Ok(()),
            Some(Action::Send(message)) => {
                let sent = self.send(record, message, "deadline passed").await;
                if let Status::Funding { cancel_sent, .. } = &mut status {
                    *cancel_sent = record.sent.cancel;
                }
                sent
            }
            Some(Action::Spend(spend)) => {
                let broadcast = self.spend(record, terms, script, spend, chain).await;
                if let Ok(Some((txid, paid))) = &broadcast
                    && let Status::Claiming {
                        claim, confs, fee, ..
                    } = &mut status
                {
                    *claim = Some(txid.clone());
                    *confs = 0;
                    *fee = *paid;
                }
                broadcast.map(drop)
            }
        };
        record.status = status;
        applied
    }

    async fn send(
        &self,
        record: &mut SwapRecord,
        message: Message,
        reason: &str,
    ) -> Result<(), DriverError> {
        let id = record.swap_id;
        let client = &self.core.client;
        let limit = self.config.server_timeout;
        let wallet = self.core.ctx.wallet();
        if message == Message::Preimage {
            // Tried from before the request: a crash or an error after it may
            // still have delivered the preimage, and then only a claim gets the
            // opening.
            record.sent.preimage = true;
            self.core.store.save(record).await?;
        }
        let result = match message {
            Message::Cancel => {
                let req = CancelRequest {
                    cancel_message: reason.into(),
                };
                rt::bounded(limit, client.cancel(id, &req)).await
            }
            Message::RefundKey => {
                let key = wallet.source_refund_privkey(&id).map_err(wallet_error)?;
                let req = RevealClaimRequest {
                    source_refund_privkey: hex::encode(key.secret_bytes()),
                };
                rt::bounded(limit, client.reveal_claim(id, &req)).await
            }
            Message::Preimage => {
                let preimage = wallet.preimage(&id).map_err(wallet_error)?;
                let req = RevealPreimageRequest {
                    preimage: hex::encode(preimage),
                };
                rt::bounded(limit, client.reveal_preimage(id, &req)).await
            }
        };
        match result {
            Ok(()) => {}
            Err(e) if e.transition_rejected() => {}
            Err(_) if message == Message::Preimage => {}
            Err(e) => return Err(e.into()),
        }
        *match message {
            Message::Cancel => &mut record.sent.cancel,
            Message::RefundKey => &mut record.sent.refund_key,
            Message::Preimage => &mut record.sent.preimage,
        } = true;
        Ok(())
    }

    /// Broadcasts the claim, returning its txid and fee, or `None` when the
    /// spend it replaces stands.
    async fn spend(
        &self,
        record: &SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        spend: Spend,
        chain: &Registered,
    ) -> Result<Option<(String, Option<u64>)>, DriverError> {
        let Spend::Claim {
            utxo,
            path,
            fee_rate,
            replaces,
        } = spend;
        let id = record.swap_id;
        let wallet = self.core.ctx.wallet();
        let ours = wallet.dest_claim_privkey(&id).map_err(wallet_error)?;
        let destination = record
            .endpoint(terms.side)
            .destination
            .as_deref()
            .unwrap_or_default();
        let utxos = [utxo];
        let tx_hex = match path {
            Path::Keypath { server_privkey } => {
                script.keypath(&utxos, ours, server_privkey, destination, fee_rate)?
            }
            Path::Preimage => {
                let preimage = wallet.preimage(&id).map_err(wallet_error)?;
                script.claim_by_preimage(&utxos, ours, preimage, destination, fee_rate)?
            }
        };
        match chain.client.broadcast(&tx_hex).await {
            Ok(txid) => Ok(Some((txid, built_fee(terms, script, &tx_hex, &utxos)))),
            Err(_) if replaces.is_some() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Refunds the `Refundable` funds in one transaction, the most valuable
    /// first and at most `FUNDS_PER_PASS`, once no refund of ours is left in
    /// the mempool, so the rest go after it is in a block: to `destination`,
    /// else the swap's refund address, else the hook's, asked only now that
    /// there is something to take. By keypath while `refund_keypath` holds
    /// the server's key, by CSV without.
    async fn refund_funds(
        &self,
        record: &mut SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        observed: &Observed,
        chain: &Registered,
        destination: Option<&str>,
    ) -> Result<Option<String>, DriverError> {
        if record
            .funds
            .iter()
            .any(|fund| matches!(fund.state, FundState::Refunding { .. }))
        {
            return Ok(None);
        }
        let mut refundable: Vec<_> = observed
            .outputs
            .iter()
            .filter(|output| {
                record.funds.iter().any(|fund| {
                    fund.outpoint == output.utxo.outpoint() && fund.state == FundState::Refundable
                })
            })
            .collect();
        refundable.sort_by_key(|output| Reverse(output.value));
        let utxos: Vec<SwapUtxo> = refundable
            .into_iter()
            .take(FUNDS_PER_PASS)
            .map(|output| output.utxo.clone())
            .collect();
        if utxos.is_empty() {
            return Ok(None);
        }
        let own = &record.endpoint(terms.side).destination;
        let destination = match (destination, own, &self.destinations) {
            (Some(given), _, _) => given.to_owned(),
            (None, Some(own), _) => own.clone(),
            (None, None, Some(destinations)) => destinations
                .refund_address(terms.chain)
                .await
                .map_err(DriverError::Destination)?,
            (None, None, None) => return Ok(None),
        };
        let fee_rate = observed.fee_rate.min(chain.config.max_fee_rate);
        let ours = self
            .core
            .ctx
            .wallet()
            .source_refund_privkey(&record.swap_id)
            .map_err(wallet_error)?;
        let tx_hex = match record.refund_keypath() {
            Some(server) => script.keypath(&utxos, server, ours, &destination, fee_rate)?,
            None => script.refund_by_csv(&utxos, ours, &destination, fee_rate)?,
        };
        let txid = chain.client.broadcast(&tx_hex).await?;
        let fee = built_fee(terms, script, &tx_hex, &utxos);
        for fund in &mut record.funds {
            if utxos.iter().any(|utxo| utxo.outpoint() == fund.outpoint) {
                fund.state = FundState::Refunding {
                    txid: txid.clone(),
                    fee,
                };
            }
        }
        Ok(Some(txid))
    }
}

/// The fee of a spend of `utxos` just built, before any pass observes it.
fn built_fee(terms: &Terms, script: &SwapScript, tx_hex: &str, utxos: &[SwapUtxo]) -> Option<u64> {
    let tx = Tx::decode(terms.chain, tx_hex).ok()?;
    tx.fee(|prev| {
        let utxo = utxos.iter().find(|utxo| utxo.outpoint() == *prev)?;
        script.value(utxo)
    })
}
