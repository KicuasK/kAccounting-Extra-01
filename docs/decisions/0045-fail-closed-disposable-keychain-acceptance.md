# ADR 0045: Fail-closed disposable Keychain acceptance

- Status: Accepted technical safety boundary; real OS acceptance remains blocked
- Date: 2026-10-22

## Context

The remaining acceptance includes real macOS Keychain behavior. Development may
not access existing financial keys, mutate default/login keychains or change the
user's search list. The cached security-framework 3.7.0 creation wrapper calls
SecKeychainCreate directly. Neither that wrapper nor the installed SDK header
establishes search-list isolation. A temporary path alone is insufficient proof.

## Decision

Add a pure opt-in preflight that returns a typed blocker before any OS calls.
Even explicit opt-in cannot bypass `IsolationUnverified`. Keep the optional
integration test ignored by default and make an attempted acceptance fail
honestly while blocked. Validate device and recipient contracts using a shared
memory fake, including create-only behavior, namespace separation, malformed
secrets and locked/unlocked preservation. Do not modify production adapters.

No actual Keychain item is created/read/deleted, no keychain is created, and no
default/search-list setting is changed. No password policy is introduced.

## Consequences

Memory contracts improve regression coverage, but do not qualify as real OS
Keychain acceptance. A documented isolated creation path or separately approved
disposable macOS user/VM is required before real acceptance can be completed.
See `docs/KEYCHAIN_ACCEPTANCE.md` for exact commands and remaining scope.
