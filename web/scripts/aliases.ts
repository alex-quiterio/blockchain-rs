// Lets Node resolve the layer aliases, so tests run the sources as they are:
//   node --import ./web/scripts/aliases.ts --test ...
// `@domain/transfers` becomes web/src/domain/transfers.ts, which Node runs by stripping its types.

import { registerHooks } from "node:module";
import { unalias } from "./layers.ts";

const SRC = new URL("../src/", import.meta.url);

registerHooks({
  resolve(specifier, context, next) {
    const path = unalias(specifier);
    return next(path ? new URL(`${path}.ts`, SRC).href : specifier, context);
  },
});
