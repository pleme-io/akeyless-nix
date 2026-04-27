# akeyless-nix

> **★★★ CSE / Knowable Construction.** This repo operates under **Constructive Substrate Engineering** — canonical specification at [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). The Compounding Directive (operational rules: solve once, load-bearing fixes only, idiom-first, models stay current, direction beats velocity) is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before non-trivial changes.


Drop-in replacement for `sops-nix` that fetches secrets from Akeyless instead of
decrypting a git-committed SOPS file. Secrets are pulled via the Akeyless API at
activation time and written to files with proper permissions -- identical interface
to sops-nix for zero-migration-friction.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Nix Module Layer                          │
│  (home-manager / nix-darwin / NixOS)                        │
│                                                              │
│  Shared lib:  module/lib.nix  (submodules, manifest builder)│
│  HM:          module/home-manager.nix  (activation script)  │
│  Darwin:      module/darwin.nix  (+ launchd agent)          │
│  NixOS:       module/nixos.nix  (+ systemd restart/reload)  │
│                                                              │
│  Generates: manifest.json  ->  Nix store derivation         │
│  Wires: activation script / launchd agent / systemd units   │
└───────────────────────┬─────────────────────────────────────┘
                        │  nix rebuild / switch
                        ▼
┌─────────────────────────────────────────────────────────────┐
│             Installer (central orchestrator)                  │
│                                                              │
│  1. Load manifest.json (manifest.rs)                         │
│  2. Load config ~/.config/akeyless-nix/ (config.rs)          │
│  3. Authenticate to Akeyless API (auth.rs)                   │
│  4. Fetch all secrets via SecretProvider trait (fetch.rs)     │
│  5. Render templates -- pure, no I/O (template.rs)           │
│  6. Write generation directory (generation.rs + write.rs)    │
│  7. Switch symlinks atomically (generation.rs)               │
│  8. Prune old generations (generation.rs)                    │
│  9. Cache secrets for offline fallback (cache.rs)            │
└─────────────────────────────────────────────────────────────┘
```

## Why Replace sops-nix?

| Concern | sops-nix | akeyless-nix |
|---------|----------|-------------|
| Secret storage | Encrypted file in git | Akeyless cloud (DFC zero-knowledge) |
| Key management | age key file on disk | API auth (no key files) |
| Access control | All-or-nothing (have key = see everything) | Per-path, per-role, per-auth-method |
| Audit | git blame | Full audit log (who, when, what, from where) |
| Update flow | Edit sops file, commit, push, rebuild | `akeyless update-secret-val`, rebuild |
| Team scaling | Share age key (risky) | Add auth methods with scoped roles |
| Rotation | Manual | API call (automatic on paid tier) |
| Offline support | Yes (local file) | No (requires API access) -- cached fallback |
| CI/CD | Needs age key in pipeline | JWT auth -- zero stored credentials |

## Compatibility with sops-nix

```nix
# sops-nix (before)
sops.secrets."github/token" = {
  path = "${homeDir}/.config/github/token";
  mode = "0600";
};

# akeyless-nix (after)
akeyless.secrets."/pleme/github/token" = {
  path = "${homeDir}/.config/github/token";
  mode = "0600";
};
```

Key difference: secret names are Akeyless paths (e.g., `/pleme/github/token`)
instead of YAML keys (e.g., `github/token`).

## Rust Binary Architecture

### Source Layout

```
src/
├── main.rs            # clap CLI: install, check, validate subcommands
├── installer.rs       # Installer -- central orchestrator composing all traits
├── manifest.rs        # Manifest + SecretSpec + TemplateSpec parsing
├── config.rs          # Config loading via shikumi (XDG discovery + AKEYLESS_NIX_* env overlay)
├── auth.rs            # Akeyless authentication (read creds, POST /auth)
├── client.rs          # AkeylessClient -- SecretProvider impl (POST /get-secret-value)
├── fetch.rs           # fetch_all() -- iterate secrets via SecretProvider trait
├── template.rs        # Placeholder substitution: <AKEYLESS:hash:PLACEHOLDER> -> value
├── generation.rs      # Generation lifecycle: create, switch symlinks, prune
├── write.rs           # File writing with permissions + ownership (chown)
├── cache.rs           # FsCache -- CacheStore impl for offline fallback
├── traits.rs          # Trait definitions: SecretProvider, FileWriter, CacheStore
└── platform/
    ├── mod.rs         # Platform-conditional compilation
    ├── darwin.rs      # macOS-specific (reserved)
    └── linux.rs       # Linux-specific (reserved)
```

### Trait Hierarchy

All I/O boundaries are abstracted behind traits for testability:

- **`SecretProvider`** -- fetch a secret value by Akeyless path.
  Auth is a separate concern (`auth.rs`), not part of this trait.
  Impls: `AkeylessClient` (real), test mocks.

- **`FileWriter`** -- directory creation, file removal, symlink creation.
  Used by generation management. Secret file writing (with permissions and
  ownership) is handled separately by `write::write_secret_with_ownership`.
  Impls: `FsFileWriter` (real), test mocks.

- **`CacheStore`** -- persist and load secret maps for offline fallback.
  Impls: `FsCache` (real), test mocks.

### Installer Flow

The `Installer` struct is the central service that composes all dependencies:

```
Installer::new(provider: &dyn SecretProvider, cache: Option<&dyn CacheStore>)
  .install(manifest, ignore_passwd)
    1. Fetch secrets (with cache fallback on failure)
    2. Render templates (pure computation)
    3. Create generation directory, write secret + template files
    4. Switch symlinks atomically
    5. Prune old generations
    6. Cache secrets (non-fatal)
```

### Config File

`~/.config/akeyless-nix/akeyless-nix.yaml` (falls back to defaults if absent):

```yaml
auth:
  access_id_file: ~/.config/akeyless/access-id
  access_key_file: ~/.config/akeyless/access-key
api_url: https://api.akeyless.io
cache:
  enabled: true
  dir: ~/.cache/akeyless-nix
  ttl_seconds: 3600
```

### Manifest Format

Generated by Nix module at build time, consumed at activation time:

```json
{
  "secrets": [
    {
      "akeyless_path": "/pleme/github/token",
      "file_path": "/Users/luis/.config/github/token",
      "mode": "0600",
      "owner": "",
      "group": "",
      "uid": null,
      "gid": null,
      "restart_units": [],
      "reload_units": []
    }
  ],
  "templates": [
    {
      "name": "kubeconfig",
      "content": "token: <AKEYLESS:sha256hash:PLACEHOLDER>",
      "file_path": "/Users/luis/.kube/credentials",
      "mode": "0600"
    }
  ],
  "generations_dir": "/Users/luis/.local/share/akeyless-nix/generations",
  "symlink_path": "/Users/luis/.local/share/akeyless-nix/secrets",
  "keep_generations": 2
}
```

## Nix Module Architecture

### Shared Library (`module/lib.nix`)

Common definitions used by all three modules:

- `secretSubmodule` / `templateSubmodule` -- option type definitions
- `effectiveSecretPath` / `effectiveTemplateContent` -- path resolution helpers
- `buildManifest` -- generates the JSON manifest derivation
- `mkPlaceholders` -- generates placeholder map for template substitution
- `collectRestartUnits` / `collectReloadUnits` -- unit aggregation

### Home-Manager Module

- Activation script runs after `writeBoundary`
- Failure is non-fatal (logged warning) to avoid bricking rebuilds offline
- Uses `ignorePasswd` option for CI/dry-run contexts

### Darwin Module

- Imports home-manager module
- Adds launchd agent (`io.pleme.akeyless-nix`) for boot/login-time refresh
- Copies manifest to stable path so launchd can reference it
- Respects `ignorePasswd` in both activation script and launchd agent

### NixOS Module

- Secrets written to `/run/akeyless-nix/` (in-memory)
- Supports `owner`/`group` with `root` defaults (vs empty for HM)
- `neededForUsers` flag for pre-sysusers secret decryption
- `restartUnits`/`reloadUnits` for systemd service integration
- Activation runs after `specialfs`, `users`, `groups`

## Nix Integration

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    substrate.url = "github:pleme-io/substrate";
  };
  outputs = { self, nixpkgs, substrate, ... }:
    (import "${substrate}/lib/rust-tool-release-flake.nix" { ... })
    {
      toolName = "akeyless-install-secrets";
      src = self;
      repo = "pleme-io/akeyless-nix";
    } // {
      homeManagerModules.default = import ./module;
      darwinModules.default = import ./module/darwin.nix;
      nixosModules.default = import ./module/nixos.nix;
    };
}
```

## Bootstrap Problem

The chicken-and-egg: akeyless-nix needs credentials to fetch secrets,
but the credentials themselves might be secrets.

**Solution:** Bootstrap credentials are provisioned manually once:

```bash
mkdir -p ~/.config/akeyless
echo "p-nn5huxl36myiam" > ~/.config/akeyless/access-id
echo "secret-key-here" > ~/.config/akeyless/access-key
chmod 600 ~/.config/akeyless/*
```

After first activation, akeyless-nix manages everything else.

## Testing Strategy

50 tests covering:
- **Unit tests** with mock `SecretProvider` and `CacheStore` (no API calls)
- **Integration tests** exercising full install flow with mocks
- **Config tests** -- YAML loading, defaults, path expansion
- **Manifest tests** -- parsing, uid/gid, long paths, empty manifests
- **Template tests** -- placeholder substitution, passthrough, newlines, special chars
- **Generation tests** -- lifecycle, pruning, empty manifests, nonexistent dirs
- **Cache tests** -- store/load cycle, permissions, missing cache
- **Installer tests** -- full flow, cache fallback, empty manifests, check command

All tests use `ignore_passwd = true` to avoid requiring root or specific system users.

## Edge Cases Handled

- Manifest with 0 secrets and 0 templates: creates empty generation, switches symlink
- Secret values with newlines or special characters: passed through verbatim
- `generations_dir` does not exist yet: created automatically
- Prune with fewer generations than `keep_generations`: no-op
- API failure with cache: falls back to cached secrets with warning
- API failure without cache: hard fail
- Empty owner/group strings: skip chown entirely (-1 uid/gid)
