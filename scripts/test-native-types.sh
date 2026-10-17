#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
docker compose -f compose.yaml -f compose.test.yaml --profile test build test-postgres test-mysql test-sqlite
for backend in postgres mysql sqlite; do
    docker compose -f compose.yaml -f compose.test.yaml --profile test run --rm "test-$backend"
done
