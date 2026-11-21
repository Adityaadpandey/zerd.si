# An unrelated type error that is already there before the agent starts.
mkdir -p apps/server/src
cat > apps/server/src/__bench_broken.ts <<'EOF'
// Unrelated legacy constant. Owned by another team, do not change in feature work.
export const legacyRetryLimit: number = "three";
EOF
