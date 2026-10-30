// The layer aliases, the one list the build and Node both read. web/tsconfig.json lists them too,
// for TypeScript.

export const LAYERS: Record<string, string> = {
  "@domain/": "domain/",
  "@application/": "application/",
  "@infra/": "infrastructure/",
  "@ui/": "ui/",
};

/**
 * The path under the source root that an aliased specifier names, without an extension; or null.
 */
export function unalias(specifier: string): string | null {
  for (const [alias, folder] of Object.entries(LAYERS)) {
    if (specifier.startsWith(alias))
      return folder + specifier.slice(alias.length);
  }
  return null;
}
