import type { SidebarsConfig } from "@docusaurus/plugin-content-docs";

const sidebars: SidebarsConfig = {
  guide: [
    "overview",
    "quickstart",
    "swaps",
    "driving",
    "refunds",
    "recovery",
    "liquid",
    "errors",
    "configuration",
    "platforms",
    "boltz",
    {
      type: "category",
      label: "API reference",
      items: [
        { type: "link", label: "Rust", href: "https://docs.rs/anyswap-sdk" },
        {
          type: "category",
          label: "TypeScript",
          link: { type: "doc", id: "typescript/reference/Reference" },
          items: [
            "typescript/reference/Api",
            "typescript/reference/Context",
            "typescript/reference/Driver",
            "typescript/reference/Esplora",
            "typescript/reference/Types",
            "typescript/reference/Utils",
            "typescript/reference/Wallet",
          ],
        },
      ],
    },
  ],
};

export default sidebars;
