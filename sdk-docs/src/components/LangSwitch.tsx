import { useEffect } from "react";
import { LANGS, type Lang, useLang } from "./Lang";
import styles from "./LangSwitch.module.css";

/** The navbar's language toggle. */
export default function LangSwitch({ mobile }: { mobile?: boolean }) {
  const { lang, query, stored, setLang } = useLang();

  // A `?lang=` link sets the language for the pages read after it too.
  useEffect(() => {
    if (query && query !== stored) setLang(query);
  }, [query, stored]);

  return (
    <div className={`${styles.switch} ${mobile ? "menu__list-item" : "navbar__item"}`}>
      {(Object.keys(LANGS) as Lang[]).map((value) => (
        <button
          key={value}
          type="button"
          className={value === lang ? `${styles.option} ${styles.active}` : styles.option}
          aria-pressed={value === lang}
          onClick={() => setLang(value)}
        >
          {LANGS[value]}
        </button>
      ))}
    </div>
  );
}
