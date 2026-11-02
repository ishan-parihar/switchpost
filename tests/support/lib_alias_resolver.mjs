// Node module customization hook: maps `$lib/...` to frontend/src/lib/...
// and any `.svelte` import to the theme shim.
//
// WHY THIS EXISTS. `frontend/src/lib/providers.ts` imports `theme` from
// `$lib/stores/theme.svelte`. That is a Vite alias (resolved by the bundler)
// and a `.svelte` file (compiled by Vite), so running the module under node
// fails at load. Several Rust integration tests load this module to assert
// the REAL curated provider table rather than a hand-written second copy of
// it (AGENTS.md 9.18).
//
// This hook keeps that property without changing production source to suit
// a test tool. Registered via `--import`, per node's customization-hooks API.
import { register } from 'node:module';

register(new URL('./resolve_hook.mjs', import.meta.url));
