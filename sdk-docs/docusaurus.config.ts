import type { Config } from "@docusaurus/types";
import type * as Preset from "@docusaurus/preset-classic";
import { existsSync } from "node:fs";
import { themes as prismThemes } from "prism-react-renderer";

// The per-language pages the guide replaced, still linked from the READMEs of
// published SDK versions, with the page and language that replaced each.
const moved: [from: string, page: string, lang: "rust" | "ts"][] = [
  ["rust/overview", "overview", "rust"],
  ["rust/swaps", "swaps", "rust"],
  ["rust/recovery", "recovery", "rust"],
  ["rust/liquid", "liquid", "rust"],
  ["rust/boltz", "boltz", "rust"],
  ["typescript/overview", "overview", "ts"],
  ["typescript/driver", "swaps", "ts"],
  ["typescript/liquid", "liquid", "ts"],
  ["typescript/recovery", "recovery", "ts"],
];

const redirects = moved.map(([from, page, lang]) => {
  if (!existsSync(`docs/${page}.mdx`)) {
    throw new Error(`/${from} redirects to ${page}, which is gone`);
  }
  // Relative, since the plugin drops the trailing slash before a query.
  return { from: `/${from}`, to: `../../${page}/?lang=${lang}` };
});

const config: Config = {
  title: "AnySwap SDKs",
  tagline: "Swaps between Lightning and on-chain Bitcoin or Liquid",
  url: "https://blockstream.github.io",
  baseUrl: "/anyswap/",
  trailingSlash: true,
  favicon: "img/favicon.ico",

  onBrokenLinks: "throw",
  onBrokenAnchors: "throw",
  markdown: {
    // Plain .md pages are CommonMark, so a `<` or `{` in prose is text, not MDX.
    format: "detect",
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: "throw",
    },
  },
  themes: ["@docusaurus/theme-mermaid"],
  plugins: [["@docusaurus/plugin-client-redirects", { redirects }]],

  presets: [
    [
      "classic",
      {
        docs: {
          routeBasePath: "/",
          sidebarPath: "./sidebars.ts",
        },
        blog: false,
        theme: {
          customCss: "./src/css/custom.css",
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: "AnySwap SDKs",
      logo: { alt: "Blockstream", src: "img/logo.png" },
      items: [
        { type: "docSidebar", sidebarId: "guide", label: "Guide", position: "left" },
        { type: "custom-langSwitch", position: "right" },
        { href: "https://github.com/Blockstream/anyswap", label: "GitHub", position: "right" },
      ],
    },
    footer: {
      links: [
        { label: "GitHub", href: "https://github.com/Blockstream/anyswap" },
        { label: "crates.io", href: "https://crates.io/crates/anyswap-sdk" },
        { label: "npm", href: "https://www.npmjs.com/package/@blockstream/anyswap-sdk" },
        { label: "docs.rs", href: "https://docs.rs/anyswap-sdk" },
        { label: "Releases", href: "https://github.com/Blockstream/anyswap/releases" },
      ],
      copyright: "Copyright © 2026 Blockstream Corporation. MIT License.",
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.vsDark,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
