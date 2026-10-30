// Runs after tsc has compiled web/src into public/js. Browsers can't resolve the layer aliases,
// so each one becomes a relative path to the compiled module; each stylesheet is copied next to
// the module it belongs to, where the pages link it; and each page's HTML is written to public/,
// where the node serves it.

import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative } from "node:path";
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

const styles = filesUnder(SRC).filter((f) => f.endsWith(".css"));
for (const file of styles) {
  const to = join(OUT, relative(SRC, file));
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(file, to);
}

for (const [page, served] of Object.entries(PAGES)) {
  copyFileSync(join(SRC, page), join(PUBLIC, served));
}

console.log(
  `Linked the layer aliases in ${rewritten} modules, copied ${styles.length} stylesheets into public/js, and wrote ${Object.keys(PAGES).length} pages into public.`,
);
