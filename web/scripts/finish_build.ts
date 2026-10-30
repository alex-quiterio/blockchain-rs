// Runs after tsc has compiled web/src into public/js. Browsers can't resolve the layer aliases,
// so each one becomes a relative path to the compiled module; and each stylesheet is copied
// next to the module it belongs to, where the pages link it.

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
const OUT = fileURLToPath(new URL("../../public/js/", import.meta.url));

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

console.log(
  `Linked the layer aliases in ${rewritten} modules and copied ${styles.length} stylesheets into public/js.`,
);
