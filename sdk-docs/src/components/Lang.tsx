import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { useQueryString, useStorageSlot } from "@docusaurus/theme-common";
import TabItem from "@theme/TabItem";
import Tabs from "@theme/Tabs";

export const LANGS = { rust: "Rust", ts: "TypeScript" } as const;
export type Lang = keyof typeof LANGS;

const GROUP = "lang";
// Where Docusaurus keeps a tab group's choice, so names follow the code tabs.
const STORAGE_KEY = `docusaurus.tab.${GROUP}`;

const isLang = (value: unknown): value is Lang =>
  typeof value === "string" && Object.hasOwn(LANGS, value);

/** The reader's language: the link's `?lang=`, else the last one picked, else Rust. */
export function useLang(): {
  lang: Lang;
  query: Lang | null;
  stored: Lang | null;
  setLang: (lang: Lang) => void;
} {
  const [queryValue, setQuery] = useQueryString(GROUP);
  const [storedValue, slot] = useStorageSlot(STORAGE_KEY);
  const query = isLang(queryValue) ? queryValue : null;
  const stored = isLang(storedValue) ? storedValue : null;
  const setLang = (lang: Lang) => {
    slot.set(lang);
    if (query) setQuery(lang);
  };
  return { lang: query ?? stored ?? "rust", query, stored, setLang };
}

/** An identifier in the reader's language. */
export function Id({ rust, ts }: { rust: string; ts: string }) {
  const { lang } = useLang();
  return <code>{lang === "ts" ? ts : rust}</code>;
}

export function Rust({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

export function Ts({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

/** One snippet per language, switched together with every other on the site. */
export function Code({ children }: { children: ReactNode }) {
  const snippets = Children.toArray(children).filter(isValidElement) as ReactElement<{
    children: ReactNode;
  }>[];
  return (
    <Tabs groupId={GROUP} queryString>
      {snippets.map((snippet) => {
        const lang: Lang = snippet.type === Ts ? "ts" : "rust";
        return (
          <TabItem key={lang} value={lang} label={LANGS[lang]}>
            {snippet.props.children}
          </TabItem>
        );
      })}
    </Tabs>
  );
}
