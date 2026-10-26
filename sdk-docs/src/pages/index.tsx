import Link from "@docusaurus/Link";
import CodeBlock from "@theme/CodeBlock";
import Layout from "@theme/Layout";
import styles from "./index.module.css";

const sdks = [
  {
    name: "Rust",
    install: "cargo add anyswap-sdk --features esplora",
    guide: "/quickstart/?lang=rust",
    reference: "https://docs.rs/anyswap-sdk",
  },
  {
    name: "TypeScript",
    install: "npm install @blockstream/anyswap-sdk",
    guide: "/quickstart/?lang=ts",
    reference: "/typescript/reference/",
  },
];

export default function Home() {
  return (
    <Layout description="SDKs for swaps between Lightning and on-chain Bitcoin or Liquid">
      <header className={styles.hero}>
        <h1>AnySwap SDKs</h1>
        <p>
          Swaps between Lightning and on-chain Bitcoin or Liquid. Each SDK carries a
          driver that runs any number of swaps to completion and verifies everything
          against the chain, so an integration creates swaps and watches statuses
          rather than implementing the protocol.
        </p>
      </header>
      <main className={styles.sdks}>
        {sdks.map((sdk) => (
          <section key={sdk.name} className={styles.sdk}>
            <h2>{sdk.name}</h2>
            <CodeBlock language="bash">{sdk.install}</CodeBlock>
            <div className={styles.links}>
              <Link className="button button--primary" to={sdk.guide}>
                Get started
              </Link>
              <Link className="button button--secondary button--outline" to={sdk.reference}>
                API reference
              </Link>
            </div>
          </section>
        ))}
      </main>
    </Layout>
  );
}
