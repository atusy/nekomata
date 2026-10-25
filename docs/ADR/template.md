# Title Stating the Decision

<!--
File naming: `YYYYMMDD-HHMMSS-slug.md`, where the slug restates the title.

Scope: an ADR records an *architecture* decision — which component owns what,
which boundaries exist and which way dependencies point. It does not specify
behavior. Observable behavior (formats, defaults, precedence, limits, error
handling) belongs in `docs/spec/`; how to use nekomata belongs in
`docs/manual/`. Before writing a statement, ask: could a competent implementer
change it while keeping every component's responsibility intact? If yes, it is
spec or mechanism, not decision.

An ADR is a decision, not a description of the current code. Do not rewrite it
to follow the implementation. When the code diverges, state the divergence in
"Decision–Implementation Gap". When the decision itself changes, write a new
ADR and delete the old file.

History is NOT recorded here. No status tables, implementation dates,
validation logs, completion checklists or "renamed from" notes: every file in
this directory is in force, and its history lives in `git log --follow <file>`.

Refer to other ADRs by their slug in plain text in body prose. Markdown links
to ADRs belong only in a final "Related Decisions" list, so that deleting a
superseded record leaves at most one stale link.
-->

## Context

What forces make this decision necessary? What constraints apply?

## Decision

What is decided, stated as responsibilities and boundaries.

## Invariants

<!-- Optional. Include when the decision carries traps an implementation must
not fall into. Open with this note, verbatim:

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

Then list each one in one or two sentences: what must hold and why. -->

## Considered Options

Alternatives that were genuinely evaluated, and why each was or was not chosen.

## Consequences

What becomes easier or harder as a result? Include negative consequences.

## Decision–Implementation Gap

<!-- Optional. Include ONLY when the implementation diverges from this
decision: what is deferred, partial or done differently. -->

## Related Decisions
