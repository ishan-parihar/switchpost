// The `$lib/stores/theme.svelte` shim for node.
//
// `frontend/src/lib/providers.ts` imports `theme` from a Vite-alias path
// that node cannot resolve (`$lib/...`) and a `.svelte` file it cannot
// compile. Several Rust integration tests (google_family_glyph_test) load
// `providers.ts` under node to test the REAL curated table rather than a
// hand-written copy of it (AGENTS.md 9.18).
//
// The real store's shape is what matters: a `.value` the ink helpers read
// to pick ink per surface. Everything else in the store is irrelevant to
// those tests, so this shim carries only the contract they depend on.
//
// Keep in sync: if `theme.svelte` grows a field the ink paths read, this
// shim needs it. A test that silently measures the shim instead of the
// store is the failure mode to avoid.

export const theme = { value: "dark" };
