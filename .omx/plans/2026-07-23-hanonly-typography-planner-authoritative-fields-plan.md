# HanOnly Typography Planner authoritative fields remediation

> Status: G005 Execution Card - implementation verified, checkpoint pending.
>
> Parent authority: `.omx/plans/2026-08-05-hanonly-functional-delivery-plan.md`.
> This card is subordinate to that sole active plan, cannot complete G005 by
> itself, and does not reopen or modify completed G004.

## Scope

Repair the recurring HanOnly Typography Planner fallback by making the
application translation and explicit-size policy authoritative. Automatic size
remains renderer-owned. Limit production changes to
`crates/koharu-app/src/typography.rs` and its colocated tests.

## Supersession boundary

This card supersedes only the HanOnly semantic-rejection rules in
`.omx/plans/2026-07-19-source-relative-target-typography-plan.md` that require:

- exact returned `lines` and rejection of any changed or reflowed lines;
- rejection of every non-null returned `fontSize`;
- the corresponding rejection-test names and zero-op PASS statement; and
- the consequence statement that legacy HanOnly Planner reflow is not accepted.

This card does not modify the other 2026-07-19 contracts. Their current
applicability is determined by the parent functional-delivery plan and later
accepted work. In this slice, the prompt's exact line/null-size request,
manual-size priority, Planner ownership of the remaining validated style fields,
renderer line-count and placement safety, protected pixels, AllText behavior,
and atomic writes remain unchanged.

The apparent prompt/consumer difference is deliberate: the prompt requests the
canonical response, while the consumer tolerates schema-valid deviations only
for fields it discards. Ignored `lines` and `fontSize` cannot reach the emitted
HanOnly op; malformed JSON, wrong types, unknown fields, and every still-owned
style field remain strictly validated.

## Invariants

- HanOnly translation text is copied from `TypographyTarget.translation` without model-controlled reflow.
- HanOnly automatic font size remains `None` in the Typography op so renderer fitting owns the final size.
- HanOnly manual font size remains the highest-priority explicit size.
- AllText retains existing Planner reflow and bounded font-size behavior.
- Node coverage, identity, font allowlist, style bounds, downstream protected-Latin/sprite/geometry/overlap validation, and atomic writes remain unchanged.
- No dependencies, public types, Scene/OpenAPI fields, compatibility shims, provider changes, or UI warning changes.

## Regression contract

### 1. Authoritative HanOnly fields

The regressions require successful operation construction when a HanOnly response:

- inserts a line break;
- changes spaces or line grouping;
- rearranges `lines` for a target with multiple safe regions;
- supplies a valid finite `fontSize` or a parseable finite value that is negative, below the page readability floor, or above 300 px.

Assert the generated patch still contains the exact original translation and has no automatic explicit font size.

Extend the manual-size test so a conflicting Planner `fontSize` is ignored while the original manual size and `typography_plan_verified=false` remain unchanged.

Migrate every existing text/whitespace rejection test that currently uses HanOnly to AllText so the validation boundary remains covered. This includes changed text, empty lines, collapsed spaces, tabs, trimmed edges, and ideographic spaces. Do not delete those assertions.

Keep malformed response coverage distinct: `fontSize: 1e400`, an incorrect JSON type, or any other value that cannot deserialize into `Option<f32>` must still fail atomically before the authority branch for both policies.

The historical pre-edit RED step is not a current completion gate. The
implementation already exists in the worktree; current completion depends on
the deterministic regression and static checks below.

### 2. Minimal authority boundary

Inside `build_typography_ops`:

- Compute `(translation, planned_font_size)` once per target.
- For `preserve_lines=true`, treat every successfully deserialized `lines: Vec<String>` as a non-authoritative suggestion, including empty, changed, reflowed, or multi-safe-region values. Skip semantic line validation and select `target.translation` as the output translation.
- For `preserve_lines=true`, treat every successfully deserialized finite `style.font_size` value as a non-authoritative suggestion, including negative, below-floor, and over-limit values. Select `planned_font_size=None`, so only `target.manual_font_size` can become explicit.
- For `preserve_lines=false`, retain `validate_lines`, numeric font-size validation, and `manual_font_size.or(planned_font_size)` behavior.
- Preserve all other response validation and style mapping.

Narrow `validate_lines` to the non-HanOnly paths it still owns. Keep its genuine multiple-safe-region and text/whitespace errors distinct for any non-HanOnly request carrying multiple regions. Do not duplicate HanOnly region-count validation here: the original application translation remains unchanged, and the existing renderer validates application-owned translation line count against eligible safe regions.

### 3. Regression matrix

- HanOnly: changed/empty lines, changed whitespace, multi-safe-region reordering, supplied font size, parseable negative/below-floor/oversized supplied font size, manual override conflict. All must retain the original translation and authoritative size policy.
- AllText: valid reflow succeeds; changed text/whitespace and invalid/oversized font size fail.
- Safety: malformed or non-deserializable response (including `fontSize: 1e400` or wrong JSON type), missing/duplicate/unknown node, unknown font, invalid stroke width, and unknown fields remain rejected atomically.

## Execution Card validation

Run only checks that prove this bounded change. The parent functional-delivery
plan owns workspace-wide, UI, CPU/Metal, and end-to-end release verification;
those checks must not be duplicated here or converted into a second completion
authority.

```sh
bun cargo test -p koharu-app typography::tests::han_only_typography -- --nocapture
bun cargo test -p koharu-app typography -- --nocapture
bun cargo test -p koharu-app pipeline -- --nocapture
bun cargo check -p koharu-app --all-targets
bun cargo fmt -p koharu-app --check
git diff --check
```

Existing workspace-wide test or Clippy baseline failures outside the changed
file are recorded separately and do not authorize adjacent cleanup in this
card. They remain visible to the parent plan's later release verification.

## Review and rollback

- One independent code review must verify the field-authority boundary, AllText
  isolation, manual-size priority, and retained atomic safety.
- Ralph completion requires one final native Architect approval of the same
  diff. A second Critic or UltraQA gate is not required for this one-file card.
- Rollback is the path-scoped reversal of the production hunk and associated tests in `typography.rs`; no data migration is involved.

## Runtime acceptance follow-up

Automated deterministic tests use local response fixtures and are sufficient to
checkpoint this card. Runtime cloud-model behavior is part of the parent
G005/G008 visual acceptance, not a second gate here. That later evidence must
identify the build SHA, fixture path and hash, policy/model configuration,
expected preserved translation and automatic/manual size outcome, and the
observed app result. No cloud request is made automatically by this card.
