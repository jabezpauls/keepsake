# Media Vault Offsite Replica

A headless Media Vault peer that stores **ciphertext only**. Designed
for running on a VPS, NAS, or spare machine as an always-on backup
target for your primary Media Vault install.

## What it does

- Boots an iroh endpoint using a self-managed Ed25519 + X25519 identity
  (persisted under `$MV_REPLICA_ROOT`).
- Prints a pairing ticket once per boot; operators paste it into their
  primary Media Vault's **Peers → Accept** flow.
- After pairing, accepts namespace tickets (one per shared album) that
  the primary's **Share → Copy ticket** flow produces.
- Pulls ciphertext blobs opportunistically as CRDT events land.
- Never holds user passwords, plaintext photos, or collection keys.

## What it does NOT do

- No UI.
- No auto-discovery — you must feed it pairing + namespace tickets.
- No plaintext ever. The replica cannot read the photos it stores.
  Recovering from the replica requires a fresh primary install that
  pairs with it and unwraps its own collection keys.

## Build

```sh
cargo build --release -p mv-replica
install -m 0755 target/release/mv-replica /usr/local/bin/mv-replica
```

## First boot

```sh
MV_REPLICA_ROOT=/var/lib/mv-replica /usr/local/bin/mv-replica --print-ticket
```

Output includes a `pairing ticket:` line. Copy the base32 string into
your primary app's **Peers → Accept** textarea. Nothing else is needed
on the replica side until the primary shares an album.

## Sharing an album to the replica

On the primary app, open the album → **Share…** → click **Share** next
to the replica peer row. The modal surfaces a **namespace ticket**.
Copy it, then hand it to the replica:

```sh
MV_REPLICA_ROOT=/var/lib/mv-replica /usr/local/bin/mv-replica \
    --accept-namespace <paste-the-base32-here>
```

Subsequent runs of `mv-replica` with no args automatically resume every
namespace you've ever accepted — `--accept-namespace` is only for the
one-time handshake.

## Running as a systemd service

Copy `scripts/replica.service` → `/etc/systemd/system/mv-replica.service`,
create the unprivileged user, then:

```sh
sudo useradd --system --home /var/lib/mv-replica --shell /usr/sbin/nologin mv-replica
sudo install -d -o mv-replica -g mv-replica /var/lib/mv-replica
sudo systemctl daemon-reload
sudo systemctl enable --now mv-replica
sudo journalctl -u mv-replica -f
```

The first boot logs the pairing ticket via `tracing`; grab it from
`journalctl` and paste it into the primary app.

## Environment variables

| Variable | Purpose | Default |
|---|---|---|
| `MV_REPLICA_ROOT` | Data directory (keystore + blobs + docs) | `$HOME/.local/share/mv-replica` |
| `RUST_LOG` | `tracing` filter | `info` |

## CLI flags

| Flag | Purpose |
|---|---|
| `--vault-root <dir>` | Override `MV_REPLICA_ROOT` |
| `--relay <url>` | Opt in to a public iroh relay (default: LAN-only) |
| `--bind-port <port>` | Pin a UDP port (default: OS-assigned) |
| `--accept-namespace <b32>` | Accept a namespace ticket at boot (repeatable) |
| `--print-ticket` | Print the pairing ticket and exit |

## Security posture

- `MV_REPLICA_ROOT` holds the replica's own Ed25519 + X25519 identities
  plus received ciphertext. An attacker with disk access learns:
  - which primaries have paired with the replica (from `peer_accept`);
  - which collection namespaces the replica has accepted;
  - the ciphertext of every asset received.
- The attacker does **not** learn:
  - any plaintext photo, EXIF, filename, or collection key — those
    live wrapped under the primary's user master key;
  - the primary's password or master key.
- The sentinel keystore password is a fixed string (see `main.rs`).
  This is intentional: there's no secret to protect at the replica
  layer, so demanding an operator-supplied password only moves the
  problem (the password would need to be stored in the systemd unit).

## Upgrading

Replica schema evolves alongside the main app. Run the new binary on
the existing `MV_REPLICA_ROOT` — migrations apply automatically via
`db::migrate::apply`. Downgrades are not supported.
