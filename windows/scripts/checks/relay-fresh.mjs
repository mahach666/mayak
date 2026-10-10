// The relay the app installs (target/release/mayak-hook.exe, see tauri.conf.json resources and
// hooks::ensure_hook_exe) must not be older than its sources: a stale one is copied into
// %LOCALAPPDATA%\Mayak\bin without a word and drops what it doesn't know yet (new agents).
// Rebuild it with `cargo build --release -p mayak-hook`.
//
// Warning only (prints WARN, exit 0) unless MAYAK_STRICT_RELAY=1, because an mtime is only a
// hint: a git checkout, a branch switch or a rebase rewrites the sources' mtimes without
// changing what they build, so a current exe can look older than them; a worktree with a
// copied exe can look either way. CI and fresh clones have no exe at all: skipped with a note.
// The exact check is the debug app's startup `mayak-hook.exe --version` against RELAY_VERSION.
import { existsSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

const root = fileURLToPath(new URL("../../", import.meta.url));
const strict = process.env.MAYAK_STRICT_RELAY === "1";
const exe = ["mayak-hook.exe", "mayak-hook"]
  .map((name) => join(root, "target", "release", name))
  .find((p) => existsSync(p));

if (!exe) {
  console.log("relay-fresh: skipped, no target/release/mayak-hook.exe (CI or fresh clone)");
  process.exit(0);
}

/** Every file under `dir`, recursively. */
function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : [join(dir, e.name)],
  );
}

const sources = [...files(join(root, "hook", "src")), join(root, "hook", "Cargo.toml"), join(root, "ident.rs")];
const newest = sources
  .map((path) => ({ path, mtime: statSync(path).mtimeMs }))
  .reduce((a, b) => (b.mtime > a.mtime ? b : a));
const built = statSync(exe).mtimeMs;
const rel = (p) => relative(root, p).replaceAll("\\", "/");

if (built >= newest.mtime) {
  console.log(`relay-fresh: ${rel(exe)} is newer than its sources`);
  process.exit(0);
}

const message =
  `relay-fresh: WARN ${rel(exe)} is older than ${rel(newest.path)} ` +
  `(${new Date(built).toISOString()} < ${new Date(newest.mtime).toISOString()}): ` +
  "rebuild it with `cargo build --release -p mayak-hook`";
if (strict) {
  console.error(message.replace("WARN", "FAIL (MAYAK_STRICT_RELAY=1)"));
  process.exit(1);
}
console.log(message);
