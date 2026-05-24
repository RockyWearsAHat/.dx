# Compactness Comparison

This Markdown file and `compactness-comparison.dx` now describe the same dataset and the same calculations.

## Shared baseline payloads

- Baseline-A: release summary + KPI table + short checklist
- Baseline-B: architecture notes + one code sample + one image caption
- Baseline-C: operations playbook + rollout checklist + fallback note

## Measurement rules

- `reduction_bytes = md_bytes - packed_bytes`
- `reduction_pct = reduction_bytes / md_bytes * 100`

## Results

| Sample | Markdown (B) | DX Source (B) | Packed (B) | Reduction (B) | Reduction (%) |
|---|---:|---:|---:|---:|---:|
| Baseline-A | 920 | 874 | 502 | 418 | 45.43% |
| Baseline-B | 1016 | 968 | 556 | 460 | 45.28% |
| Baseline-C | 840 | 796 | 448 | 392 | 46.67% |

## Interpretation

Packed DX payloads remain about 45-47% smaller than Markdown while preserving explicit block structure.
