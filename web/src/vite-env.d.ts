/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Locale used for dates and numbers; see src/format.ts. */
  readonly VITE_LOCALE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
