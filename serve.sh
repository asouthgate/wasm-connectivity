#!/bin/bash
cd "$(dirname "$0")/web"
echo "Starting Vite dev server at http://localhost:8080"
echo "  /            — solver"
echo "  /experiment  — benchmark"
npx vite --port 8080 --strictPort
