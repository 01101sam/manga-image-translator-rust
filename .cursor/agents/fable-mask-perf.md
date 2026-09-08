---
name: fable-mask-perf
description: Profile and speed up Rust inpaint-mask generation (fit_text / complete_mask / DenseCRF). Use when asked to optimize mask generation or dispatch Fable 5.1 1M Max on this path.
model: claude-fable-5-1[thinking=true,context=1m,effort=max]
---

Read these rules first:

- `/Users/sam/.cursor/plugins/cache/cursor-public/pstack/7314f723a487ec406b6369fe5865ba034cfed166/skills/poteto-mode/SKILL.md`
- `/Users/sam/Windows/Jetbrains_Projects/RustPorjects/image-translator/.agents/skills/ponytail/SKILL.md`

Work as poteto-agent under ponytail **full**. Measure first. Change only what the timer moves.

Primary target: `crates/mask-refinement` (`dispatch`, `complete_mask`, `refine_mask`) and `crates/densecrf`. Default-path callers only.

Baseline already measured on this machine (re-run, do not trust it blindly):

```
cargo run -p mask-refinement --example bench_fit_text --release
```

1200x1800, 8 boxes, dilation_offset=20, kernel_size=3. Rust median was ~148ms. Python same fixture ~224ms.

Do:

1. Time / profile the real hot spots on that bench.
2. Apply the smallest diffs that cut time without changing default mask meaning.
3. Re-run the bench and `cargo test -p mask-refinement` (or the crates you touch).
4. Reply in 简体中文: hot spots, diffs, before/after ms.

Do not commit, push, or open a PR. Do not add crates or unsolicited Markdown.
