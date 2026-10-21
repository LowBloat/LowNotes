#!/usr/bin/env bash
set -euo pipefail

# The runner keeps its registration in these files; the state volume carries
# them across container rebuilds.
state=/home/runner/state
files=(.runner .credentials .credentials_rsaparams)

case "${1:-run}" in
  register)
    # The registration token arrives on stdin, so it stays out of the compose
    # file, the environment and the host's shell history.
    read -r token
    ./config.sh --unattended \
      --url "${RUNNER_URL:?}" \
      --token "$token" \
      --name "${RUNNER_NAME:?}" \
      --labels "${RUNNER_LABELS:?}" \
      --work _work \
      --replace
    for file in "${files[@]}"; do cp "$file" "$state/"; done
    ;;
  run)
    for file in "${files[@]}"; do
      if [[ ! -f "$state/$file" ]]; then
        echo "Runner is not registered; run the register command from README.md." >&2
        exit 1
      fi
      cp "$state/$file" .
    done
    exec ./run.sh
    ;;
  *)
    exec "$@"
    ;;
esac
