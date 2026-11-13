export type Locale = "en" | "zh";

export const locales: Locale[] = ["en", "zh"];
export const defaultLocale: Locale = "en";

export const localeNames: Record<Locale, string> = {
  en: "English",
  zh: "中文",
};

export function isValidLocale(v: string): v is Locale {
  return locales.includes(v as Locale);
}
