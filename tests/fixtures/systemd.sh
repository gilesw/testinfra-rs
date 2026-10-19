#!/usr/bin/env bash
# Opt-in live fixture: creates only uniquely named, runtime systemd units.
set -euo pipefail

fixture_prefix="testinfra-rs-fixture-$$"
active_unit="${fixture_prefix}-active.service"
inactive_unit="${fixture_prefix}-inactive.service"

cleanup() {
    sudo systemctl disable --runtime --now "$active_unit" >/dev/null 2>&1 || true
    sudo rm -f "/run/systemd/system/$active_unit" "/run/systemd/system/$inactive_unit"
    sudo systemctl daemon-reload
}
trap cleanup EXIT

for unit in "$active_unit" "$inactive_unit"; do
    sudo tee "/run/systemd/system/$unit" >/dev/null <<'UNIT'
[Unit]
Description=testinfra-rs isolated test fixture
[Service]
Type=simple
ExecStart=/usr/bin/sleep infinity
[Install]
WantedBy=multi-user.target
UNIT
done
sudo systemctl daemon-reload
sudo systemctl enable --runtime --now "$active_unit"
TESTINFRA_SYSTEMD_ACTIVE_UNIT="$active_unit" \
TESTINFRA_SYSTEMD_INACTIVE_UNIT="$inactive_unit" \
    cargo test --locked --test systemd -- --ignored --nocapture
