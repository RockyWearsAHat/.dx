# Footprint Pair Analysis

This Markdown file and `footprint-pair.dx` describe the same pair dataset.

## Dataset

- Pair-A: release-note payload with one list and one quote
- Pair-B: compact tutorial payload with one table and one image caption

## Measurement rules

- `reduction_bytes = md_bytes - packed_bytes`
- `reduction_pct = reduction_bytes / md_bytes * 100`

## Results

| Sample | Markdown (B) | DX Source (B) | Packed (B) | Reduction (B) | Reduction (%) |
|---|---:|---:|---:|---:|---:|
| Pair-A | 612 | 580 | 334 | 278 | 45.42% |
| Pair-B | 488 | 462 | 271 | 217 | 44.47% |

## Combined outcome

Combined markdown bytes are 1100 and combined packed bytes are 605, yielding a 495-byte reduction (45.00%).
