#!/usr/bin/env bash
#
# One-shot deployment for the NanoHermes daemon on Debian/Ubuntu.
# Installs the binary, creates a systemd service with a Bearer token,
# and starts it. Run as root (or with sudo).
#
# Usage:
#   sudo bash deploy-debian.sh [/path/to/binary]
#
# If no binary path is given, the script builds from source (requires Rust).

set -euo pipefail

INSTALL_DIR="/opt/nanohermes"
WORKSPACE="$INSTALL_DIR/workspace"
SESSIONS_DIR="$INSTALL_DIR/sessions"
SERVICE_FILE="/etc/systemd/system/nanohermes.service"
BIN_PATH="${1:-}"

echo "==> NanoHermes daemon deployment (Debian/Ubuntu)"

# --- 1. Locate or build the binary ---
if [ -z "$BIN_PATH" ]; then
    echo "No binary given; building from source..."
    if ! command -v cargo >/dev/null 2>&1; then
        echo "Installing Rust toolchain..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    echo "Building release binary in $REPO_DIR ..."
    (cd "$REPO_DIR" && cargo build --release -p agent)
    BIN_PATH="$REPO_DIR/target/release/agent"
fi

if [ ! -f "$BIN_PATH" ]; then
    echo "ERROR: binary not found at $BIN_PATH" >&2
    exit 1
fi

# --- 2. Install binary and directories ---
echo "Installing binary to $INSTALL_DIR/nanohermes"
mkdir -p "$INSTALL_DIR" "$WORKSPACE" "$SESSIONS_DIR"
cp "$BIN_PATH" "$INSTALL_DIR/nanohermes"
chmod 755 "$INSTALL_DIR/nanohermes"
chmod 700 "$SESSIONS_DIR"

# --- 3. Generate a Bearer token if none exists ---
TOKEN_FILE="$INSTALL_DIR/.daemon_token"
if [ -f "$TOKEN_FILE" ]; then
    TOKEN="$(cat "$TOKEN_FILE")"
    echo "Reusing existing daemon token from $TOKEN_FILE"
else
    TOKEN="$(openssl rand -hex 24)"
    echo "$TOKEN" > "$TOKEN_FILE"
    chmod 600 "$TOKEN_FILE"
    echo "Generated new daemon token (saved to $TOKEN_FILE)"
fi

# --- 4. Ask for API key if not set ---
if [ -z "${NANOHERMES_API_KEY:-}" ]; then
    read -rp "Enter your LLM API key (NANOHERMES_API_KEY): " API_KEY
else
    API_KEY="$NANOHERMES_API_KEY"
fi
if [ -z "${API_KEY:-}" ]; then
    echo "ERROR: API key is required" >&2
    exit 1
fi

# --- 5. Write systemd unit ---
echo "Writing systemd unit to $SERVICE_FILE"
cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=NanoHermes Agent Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=$INSTALL_DIR
Environment="NANOHERMES_API_KEY=$API_KEY"
Environment="NANOHERMES_DAEMON_TOKEN=$TOKEN"
ExecStart=$INSTALL_DIR/nanohermes --daemon --daemon-port 8787 -C $WORKSPACE --session $SESSIONS_DIR/default.json --sandbox workspace-write
Restart=always
RestartSec=5
# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=$WORKSPACE $SESSIONS_DIR

[Install]
WantedBy=multi-user.target
EOF
chmod 600 "$SERVICE_FILE"

# --- 6. Enable and start ---
echo "Enabling and starting nanohermes.service..."
systemctl daemon-reload
systemctl enable --now nanohermes

sleep 2
if systemctl is-active --quiet nanohermes; then
    echo "==> Service is running."
else
    echo "==> WARNING: service did not start. Check: journalctl -u nanohermes -n 50"
    exit 1
fi

# --- 7. Verify ---
echo "==> Health check:"
curl -s http://127.0.0.1:8787/health && echo ""

echo ""
echo "============================================================"
echo " Deployment complete!"
echo "============================================================"
echo " Daemon token : $TOKEN"
echo " Health URL   : http://127.0.0.1:8787/health"
echo " Service      : systemctl status nanohermes"
echo " Logs         : journalctl -u nanohermes -f"
echo ""
echo " Example request:"
echo "   curl -X POST http://127.0.0.1:8787/ \\"
echo "     -H \"Authorization: Bearer $TOKEN\" \\"
echo "     -H \"Content-Type: application/json\" \\"
echo "     -d '{\"prompt\": \"list files\"}'"
echo "============================================================"
