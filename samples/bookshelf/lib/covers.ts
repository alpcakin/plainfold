const COVERS_BASE = "https://covers.openlibrary.org/b/isbn";

export function coverUrlFor(isbn?: string | null): string | null {
  if (!isbn) return null;
  const clean = isbn.replace(/[^0-9X]/gi, "");
  if (clean.length !== 10 && clean.length !== 13) return null;
  return `${COVERS_BASE}/${clean}-M.jpg`;
}
