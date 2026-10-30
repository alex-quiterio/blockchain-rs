// Runs after tsc has compiled web/src into public/js. Browsers can't resolve the layer aliases,
// so each one becomes a relative path to the compiled module; each page's stylesheets are joined
// into one, so a page links a single file however many parts it has; and each page's HTML is
// written to public/, where the node serves it.

import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { unalias } from "./layers.ts";

const SRC = fileURLToPath(new URL("../src/", import.meta.url));
const PUBLIC = fileURLToPath(new URL("../../public/", import.meta.url));
const OUT = join(PUBLIC, "js");

/** Each page's HTML, kept in the page's folder, and the path it's served at. */
const PAGES: Record<string, string> = {
  "ui/node/index.html": "index.html",
  "ui/explorer/index.html": "explorer.html",
};
/** What every page's styles start from. */
const BASE_STYLES = "ui/base.css";

const filesUnder = (dir: string): string[] =>
  readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => join(entry.parentPath, entry.name));

let rewritten = 0;
for (const file of filesUnder(OUT).filter((f) => f.endsWith(".js"))) {
  const code = readFileSync(file, "utf8");
  const linked = code.replace(
    /(\bfrom\s*|\bimport\s*\(\s*)"(@[a-z]+\/[^"]+)"/g,
    (whole, before: string, specifier: string) => {
      const path = unalias(specifier);
      if (!path) return whole;
      let to = relative(dirname(file), join(OUT, `${path}.js`))
        .split("\\")
        .join("/");
      if (!to.startsWith(".")) to = `./${to}`;
      return `${before}"${to}"`;
    },
  );
  if (linked !== code) {
    writeFileSync(file, linked);
    rewritten += 1;
  }
}

/**
 * A page's stylesheets, in the order they apply: the base, the page's main.css, then every other
 * one under its folder. Each part scopes its own rules, so the order among parts doesn't matter.
 */
function stylesOf(folder: string): string[] {
  const main = join(folder, "main.css");
  const parts = filesUnder(folder)
    .filter((f) => f.endsWith(".css") && f !== main)
    .sort();
  return [join(SRC, BASE_STYLES), main, ...parts];
}

mkdirSync(join(PUBLIC, "css"), { recursive: true });
let joined = 0;
for (const [page, served] of Object.entries(PAGES)) {
  const folder = dirname(join(SRC, page));
  const styles = stylesOf(folder);
  writeFileSync(
    join(PUBLIC, "css", `${basename(folder)}.css`),
    styles
      .map((f) => `/* ${relative(SRC, f)} */\n${readFileSync(f, "utf8")}`)
      .join("\n"),
  );
  joined += styles.length;
  copyFileSync(join(SRC, page), join(PUBLIC, served));
}

console.log(
  `Linked the layer aliases in ${rewritten} modules, joined ${joined} stylesheets into one per page in public/css, and wrote ${Object.keys(PAGES).length} pages into public.`,
);
