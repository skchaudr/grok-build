/** Canonical docs routes (app paths; Next Link applies basePath). */

export interface DocsNavItem {
  href: string;
  /** Key under dictionary.docs.sidebar */
  labelKey:
    | "quickStart"
    | "installation"
    | "configuration"
    | "architecture"
    | "features"
    | "commands"
    | "extensions"
    | "migration";
  group: "gettingStarted" | "coreConcepts" | "guides";
}

export const DOCS_NAV: DocsNavItem[] = [
  { href: "/docs/", labelKey: "quickStart", group: "gettingStarted" },
  { href: "/docs/installation/", labelKey: "installation", group: "gettingStarted" },
  { href: "/docs/configuration/", labelKey: "configuration", group: "gettingStarted" },
  { href: "/docs/architecture/", labelKey: "architecture", group: "coreConcepts" },
  { href: "/docs/features/", labelKey: "features", group: "coreConcepts" },
  { href: "/docs/commands/", labelKey: "commands", group: "coreConcepts" },
  { href: "/docs/extensions/", labelKey: "extensions", group: "coreConcepts" },
  { href: "/docs/migration/", labelKey: "migration", group: "guides" },
];

export function pathKey(path: string): string {
  return path.replace(/\/+$/, "") || "/";
}

export function docsNeighbors(pathname: string): {
  prev: DocsNavItem | null;
  next: DocsNavItem | null;
} {
  const cur = pathKey(pathname);
  const i = DOCS_NAV.findIndex((n) => pathKey(n.href) === cur);
  if (i < 0) return { prev: null, next: null };
  return {
    prev: i > 0 ? DOCS_NAV[i - 1] : null,
    next: i < DOCS_NAV.length - 1 ? DOCS_NAV[i + 1] : null,
  };
}
