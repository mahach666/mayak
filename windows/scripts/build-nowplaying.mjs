// Builds mayak-nowplaying.dylib (nowplaying/nowplaying.m) into target/release, where the app
// finds it in development and tauri.macos.conf.json takes it into the bundle. macOS only:
// elsewhere there is nothing to build.
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

if (process.platform !== "darwin") process.exit(0);

const root = fileURLToPath(new URL("..", import.meta.url));
mkdirSync(`${root}target/release`, { recursive: true });
execFileSync(
  "clang",
  [
    "-dynamiclib", "-fobjc-arc", "-O2", "-mmacosx-version-min=13.0",
    "-framework", "Foundation", "-framework", "AppKit",
    `${root}nowplaying/nowplaying.m`,
    "-o", `${root}target/release/mayak-nowplaying.dylib`,
  ],
  { stdio: "inherit" },
);
