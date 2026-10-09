//! The loop: `observe` reads the chain and the server into something
//! verified, `decide` is the pure choice of what to do, and each tick applies
//! it and saves what changed.

use anyswap_core::api::{CancelRequest, RevealClaimRequest, RevealPreimageRequest};

use super::{
    Driver, MaybeSend, MaybeSync, Registered,
    decide::{Action, Decision, Message, Path, Spend, decide, funds},
    error::DriverError,
    observe::{Observed, driven, observe},
    record::{FundState, Status, SwapRecord},
    wallet_error,
};
use crate::{
    swap_script::{SwapScript, Terms},
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
    /// what of ours is refundable, then save and announce what changed. A
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
            self.config.fee_conf_target,
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
        if applied.is_ok() {
            self.core.errors.lock().unwrap().remove(&record.swap_id);
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
        refund
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
                if let (Ok(Some(txid)), Status::Claiming { claim, confs, .. }) =
                    (&broadcast, &mut status)
                {
                    *claim = Some(txid.clone());
                    *confs = 0;
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
                client.cancel(id, &req).await
            }
            Message::RefundKey => {
                let key = wallet.source_refund_privkey(&id).map_err(wallet_error)?;
                let req = RevealClaimRequest {
                    source_refund_privkey: hex::encode(key.secret_bytes()),
                };
                client.reveal_claim(id, &req).await
            }
            Message::Preimage => {
                let preimage = wallet.preimage(&id).map_err(wallet_error)?;
                let req = RevealPreimageRequest {
                    preimage: hex::encode(preimage),
                };
                client.reveal_preimage(id, &req).await
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

    async fn spend(
        &self,
        record: &SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        spend: Spend,
        chain: &Registered,
    ) -> Result<Option<String>, DriverError> {
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
            Ok(txid) => Ok(Some(txid)),
            Err(_) if replaces.is_some() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Refunds every `Refundable` fund in one transaction: to `destination`,
    /// else the swap's refund address, else the hook's, asked only now that
    /// there is something to take. By keypath while `refund_keypath` holds the
    /// server's key, by CSV without.
    async fn refund_funds(
        &self,
        record: &mut SwapRecord,
        terms: &Terms,
        script: &SwapScript,
        observed: &Observed,
        chain: &Registered,
        destination: Option<&str>,
    ) -> Result<Option<String>, DriverError> {
        let utxos: Vec<SwapUtxo> = observed
            .outputs
            .iter()
            .filter(|output| {
                record.funds.iter().any(|fund| {
                    fund.outpoint == output.utxo.outpoint() && fund.state == FundState::Refundable
                })
            })
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
        for fund in &mut record.funds {
            if fund.state == FundState::Refundable {
                fund.state = FundState::Refunding { txid: txid.clone() };
            }
        }
        Ok(Some(txid))
    }
}
