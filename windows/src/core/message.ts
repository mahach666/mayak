// Texts Rust builds for the interface (batch spec §F): a code, its values and the English sentence
// (src-tauri/src/message.rs). No imports, so Node can check it as it is
// (C:\tmp\mayak-spike\message-check.mjs); the app hands it a translator around t().

/** Wire shape of Rust `message::Message`. Params hold paths and OS errors: never translated. */
export interface RustMessage {
  code: string;
  params: Record<string, string>;
  fallback: string;
}

/** The text of a Rust message: the translation when the dictionary knows the code, else its English fallback. */
export function messageText(
  m: RustMessage,
  tr: (key: string, vars: Record<string, string>) => string | null,
): string {
  return tr(m.code, m.params ?? {}) ?? m.fallback;
}

/** A rejected command's error is a Rust message (a `Result<_, Message>`), not a thrown Error. */
export function isRustMessage(value: unknown): value is RustMessage {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v.code === "string" && typeof v.fallback === "string";
}
