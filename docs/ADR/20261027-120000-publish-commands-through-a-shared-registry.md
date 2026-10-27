# Publish commands through a shared registry

## Context

The palette and application keymap are two entry points to the same user
operations. Maintaining separate lists of supported actions lets a palette
command become impossible to bind. Terminal operations already have shared
policies in felis, while nekomata owns the frontend in which they are invoked.

## Decision

nekomata owns a registry of public commands. A command definition owns its
stable configuration identity, palette metadata and executable action. Both
the application keymap and palette consume this registry; neither owns an
independent list of public operations. Internal execution actions are not
public commands merely because the frontend can dispatch them.

nekomata's document owns UI appearance and bindings for its public commands,
including commands that delegate to felis. felis's shared document continues
to own terminal appearance and shared terminal bindings. nekomata consumes
that document through felis-client-core and does not extend its schema.
Frontend command bindings do not duplicate the implementation of terminal
policies; they invoke the existing execution paths.

## Invariants

> The invariants below are normative; the mechanisms that satisfy them are
> deliberately unspecified.

- Defining a public command makes it available to the application keymap.
  Palette visibility may be explicitly suppressed for infrastructure commands.
- A command's stable identity is independent of its display wording.
- Invoking an operation through a key or the palette preserves the same
  execution preconditions and confirmation policy.
- Frontend configuration does not change felis's shared configuration
  vocabulary or expose unsupported upstream operations implicitly.

## Considered Options

1. **Keep independent palette and binding lists.** Rejected because every
   addition needs coordinated edits and can silently omit one entry point.
2. **Put all frontend commands in felis's schema.** Rejected because workspace,
   tab and pane commands belong to nekomata rather than the shared terminal
   client contract.
3. **Publish frontend commands from one registry while retaining shared
   terminal configuration.** Chosen. A frontend command can invoke a shared
   operation without taking ownership of its implementation or defaults.

## Consequences

- Public commands become bindable as part of their definition.
- Display wording can evolve independently of user configuration.
- Users may override shared terminal bindings in the frontend document, so
  the precedence between the two keymaps must remain explicit.
- The registry must distinguish public operations from internal execution
  messages and explicitly identify commands hidden from the palette.

## Related Decisions

- [Each input layer owns consumption and forwarding](20261025-004911-own-input-consumption-at-each-layer.md)
- [Name palette commands by category and verb phrase](20261026-230753-name-palette-commands-by-category-and-verb-phrase.md)
