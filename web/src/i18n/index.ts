import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import { readPersistedLocale, resolveLocale } from "./locale";
import { resources } from "./locales";

const initialLocale = resolveLocale(readPersistedLocale());

if (typeof document !== "undefined") {
  document.documentElement.lang = initialLocale;
}

const applyDocumentLanguage = (language: string) => {
  if (typeof document !== "undefined") {
    document.documentElement.lang = language;
  }
};

i18n.on("languageChanged", applyDocumentLanguage);

void i18n.use(initReactI18next).init({
  resources,
  lng: initialLocale,
  fallbackLng: "en",
  supportedLngs: ["en", "zh-CN"],
  defaultNS: "translation",
  ns: ["translation"],
  keySeparator: false,
  nsSeparator: false,
  interpolation: {
    escapeValue: false,
  },
  returnNull: false,
});

export { i18n };
export * from "./locale";
export default i18n;
