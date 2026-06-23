# ADR 0005: ERC-7730 clear-signing descriptor consumption

## Status

Accepted as a spike result for issue #65.

## Context

Deckard needs a path for EIP-712 and contract-call clear signing that can consume emerging ERC-7730 descriptors without making descriptor metadata part of the signing security boundary.

ERC-7730 descriptors have three useful parts for Deckard:

- `context`: binding rules that say which chain/contract or typed-data message the descriptor applies to.
- `metadata`: public project/contract details that may help orient the signer.
- `display.formats`: intent labels and field formatting rules for a specific function or message.

The Clear Signing build docs emphasize that wallets must enforce their own trust policy. Registry entries can be missing, low-quality, stale, or malicious. A wallet decides which descriptor source and review signals are acceptable before showing descriptor-enhanced signing UI.

## Decision

Deckard will use a normalized internal `ClearSigningReview` representation rather than render ERC-7730 JSON directly.

The consumption path is:

1. A resolver fetches or loads candidate ERC-7730 descriptors from a trusted source configured by Deckard.
2. The parser decodes descriptor JSON into typed structures.
3. The normalizer binds the descriptor to the reviewed data before any user-facing labels are used:
   - for calldata: `chainId` and `to`/verifying contract must match a `context.contract.deployments` entry;
   - for EIP-712 messages: the message/domain binding must match before rendering (future work).
4. The normalizer emits a small Deckard-owned review model: intent label, optional owner/contract name, and ordered field rows with labels, paths, and supported format kinds.
5. The GPUI clear-signing card renders only the normalized model, never arbitrary descriptor JSON.

Descriptor metadata is advisory display metadata only. It is not proof that a contract is safe, that a frontend is honest, or that a signature should be approved.

## Fallback behavior

Deckard must show an explicit blind/undecodable signing warning when:

- no descriptor is available;
- descriptor JSON is invalid;
- context binding fails;
- the requested function/message format is missing;
- a field format is unsupported;
- future schema features cannot be interpreted safely.

The fallback state still allows the human review flow to exist, but it must not render as a calm decoded transaction.

## Trust, versioning, and cache rules

- A descriptor source is a trust input and must be configured/reviewed as such.
- Cached descriptors need source identity, schema/version, fetch time, and invalidation behavior before production use.
- Registry poisoning and stale descriptors are expected attack paths, not edge cases.
- Interpolated intent strings must be treated carefully: unresolved or attacker-shaped substitutions must degrade to explicit field rows or fallback.

## Scope of this spike

The code added for issue #65 implements the typed descriptor subset and normalizer needed to prove the architecture:

- contract deployment context binding;
- metadata owner/contract name;
- format intent and fields;
- supported format taxonomy;
- explicit fallback states;
- fixtures and tests for descriptor-present and invalid/mismatched fallback.

It does not implement descriptor fetching, registry governance, full JSON-schema validation, EIP-712 domain matching, calldata decoding, or GPUI rendering.

## Consequences

- Deckard gets an internal seam for clear-signing descriptors without committing to a registry or schema interpreter yet.
- Future EIP-712/message-signing work can extend the same normalized model.
- Review code can stay security-oriented: bind first, render second, fall back loudly.
