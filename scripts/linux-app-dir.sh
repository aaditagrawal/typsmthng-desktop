#!/usr/bin/env bash
# Sourced by Linux packaging scripts. Expects ROOT_DIR, APP_NAME, ENV, BUILD_DIR.
# Sets APP_DIR to the Electrobun linux bundle for $ENV.

linux_bundle_name() {
  if [[ "$ENV" == "stable" ]]; then
    printf '%s\n' "$APP_NAME"
  else
    printf '%s\n' "${APP_NAME}-${ENV}"
  fi
}

ensure_linux_app_dir() {
  local bundle expected launcher
  bundle="$(linux_bundle_name)"
  expected="$BUILD_DIR/${ENV}-linux-x64/$bundle"
  launcher="$expected/bin/launcher"

  if [[ ! -f "$launcher" ]]; then
    echo "==> Missing $launcher — building Electrobun linux-x64 --env=$ENV"
    (cd "$ROOT_DIR" && bunx electrobun build --env="$ENV" --targets linux-x64)
  fi

  if [[ ! -f "$launcher" ]]; then
    echo "Error: Electrobun linux launcher not found at $launcher"
    echo "Electrobun --env=$ENV must produce build/${ENV}-linux-x64/${bundle}/bin/launcher"
    echo "Contents of $BUILD_DIR:"
    ls -la "$BUILD_DIR" 2>/dev/null || echo "  (directory does not exist)"
    exit 1
  fi

  APP_DIR="$expected"
  echo "==> Electrobun app directory: $APP_DIR"
  echo "==> Contents:"
  ls -la "$APP_DIR"
}
