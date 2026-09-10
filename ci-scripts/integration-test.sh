#!/bin/bash
set -euo pipefail

# Runs the live integration tests: the real stackql-deploy binary against
# real providers, using free resources only (AWS SSM Parameter Store standard
# parameters and GitHub repository labels). See tests/README.md.
#
# Required in the environment:
#   AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY   (AWS stacks)
#   STACKQL_GITHUB_USERNAME, STACKQL_GITHUB_PASSWORD  (GitHub stack; the
#     password is a token with issues:write on GITHUB_OWNER/GITHUB_REPO)
# Optional:
#   AWS_REGION (default us-east-1), GITHUB_OWNER, GITHUB_REPO,
#   STACKQL_DEPLOY_LIVE_RUN_ID (unique suffix for resource names; a local
#     timestamp is used when unset), STACKQL_DEPLOY_LIVE_FILTER (run only
#     tests whose name contains this string).
#
# The stackql binary must be on PATH (or in the working directory).

echo "==============================================="
echo "  Running live integration tests"
echo "==============================================="

if ! command -v stackql >/dev/null 2>&1 && [ ! -x ./stackql ]; then
  echo "error: the stackql binary was not found on PATH" >&2
  echo "       install it (https://stackql.io/downloads) or run 'stackql-deploy upgrade'" >&2
  exit 1
fi

export STACKQL_DEPLOY_LIVE_RUN_ID="${STACKQL_DEPLOY_LIVE_RUN_ID:-local-$(date +%s)}"
echo "run id: ${STACKQL_DEPLOY_LIVE_RUN_ID}"

# Each live stack starts its own stackql server on a dedicated port, but the
# provider cache under ~/.stackql is shared, so run the stacks one at a time.
cargo test --test live -- --ignored --test-threads=1 --nocapture "${STACKQL_DEPLOY_LIVE_FILTER:-}"

echo -e "\n✅ Live integration tests passed"
