import type { Locale } from "./config";
import { defaultLocale } from "./config";
import en from "./dictionaries/en";
import zh from "./dictionaries/zh";
import type { Dictionary } from "./dictionaries/en";

const dictionaries: Record<Locale, Dictionary> = { en, zh };

export function getDictionary(locale: Locale): Dictionary {
  return dictionaries[locale] ?? dictionaries[defaultLocale];
}
