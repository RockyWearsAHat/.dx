# Compactness Comparison

This document exists to compare the current .md and .dx storage paths using the same content.

> The goal is simple: ultracompact storage, fast reads, and a format that stays easy for humans and agents.

## Design targets

- Keep the visible file small.
- Keep the canonical structure explicit.
- Keep parsing deterministic.
- Keep storage compact enough to beat Markdown in practice.

## Workflow

1. Write the same material in Markdown and .dx.
2. Persist the .dx document through the real DOC database and archive path.
3. Measure file bytes, packed bytes, and shared artifact deltas.
4. Check whether the current implementation is actually winning.

## Example code

```js
export function compareSizes(markdownBytes, packedBytes) {
  return {
    markdownBytes,
    packedBytes,
    reductionBytes: markdownBytes - packedBytes,
    reductionRatio: markdownBytes === 0 ? 0 : (markdownBytes - packedBytes) / markdownBytes,
  };
}
```

## Conclusion

If .dx is going to be better than .md in every practical way, the packed representation needs to stay smaller than the plain Markdown source while the surrounding storage overhead stays controlled.
