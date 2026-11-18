# srcmap-scopes-wasm

Experimental WebAssembly binding for [`srcmap-scopes`](../../crates/scopes). The project does not publish this package.

The binding reads and writes the `scopes` and `ranges` fields of a source map, in the layout of the [ECMA-426 draft specification](https://tc39.es/ecma426/branch/proposal-scopes/).

## API

```ts
// Decode the two fields. `scopes` has one entry for each source.
decodeScopes(scopes: (string | null)[], ranges: string[], names: string[], numSources: number): ScopeInfo;

// Decode the older layout, which has one `scopes` string for the full map.
decodeLegacyScopes(scopes: string, names: string[], numSources: number): ScopeInfo;

// Encode. `scopesJson` is the JSON text of a `ScopeInfo` object.
encodeScopes(scopesJson: string, names: string[]): { scopes: (string | null)[]; ranges: string[]; names: string[] };
```

`ScopeInfo.scopes` has one list of root scopes for each source. An empty list is a source without scopes. The `definition` of a generated range is `{ sourceIndex, scopeIndex }`, or `null`. The generated `.d.ts` file has the full types.

## Tests

```bash
pnpm --filter @srcmap/scopes-wasm build
pnpm --filter @srcmap/scopes-wasm test
```

`__tests__/codec-compat.test.mjs` compares the binding with the Chrome DevTools codec ([@chrome-devtools/source-map-scopes-codec](https://jsr.io/@chrome-devtools/source-map-scopes-codec), version 2) in both directions. The codec is a development dependency.
