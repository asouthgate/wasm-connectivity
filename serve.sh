#!/bin/bash
cd "$(dirname "$0")"
bash build-wasm.sh
cd web
echo ""
echo "Starting dev server at http://localhost:8080"
echo "  /            — solver"
echo "  /experiment  — benchmark"
npx vite --port 8080 --strictPort
