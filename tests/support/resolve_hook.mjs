// The resolve hook itself, registered by lib_alias_resolver.mjs.
const LIB_ROOT = new URL('../frontend/src/lib/', import.meta.url);
const SHIM = new URL('./theme_shim.mjs', import.meta.url);

export async function resolve(specifier, context, next) {
  if (specifier.startsWith('$lib/')) {
    const rel = specifier.slice('$lib/'.length);
    if (rel.endsWith('.svelte')) return { url: SHIM.href, shortCircuit: true };
    const target = new URL(rel, LIB_ROOT);
    return { url: target.href, shortCircuit: true };
  }
  return next(specifier, context);
}
