LIB="$(cd "$(dirname "$0")/../.." && pwd)/_lib"
# /health answers 200 with status ok and the package.json version, and the secret is nowhere in the response
VERSION=$(node -p "require('./apps/server/package.json').version")
sh "$LIB/with_server.sh" sh -c '
  body=$(curl -s -w "\n%{http_code}" "$API/health")
  code=$(printf "%s" "$body" | tail -n1); json=$(printf "%s" "$body" | sed "\$d")
  [ "$code" = "200" ] || { echo "status $code"; exit 1; }
  printf "%s" "$json" | node -e "
    let s=\"\";process.stdin.on(\"data\",d=>s+=d).on(\"end\",()=>{const j=JSON.parse(s);
    if(j.status!==\"ok\")process.exit(1); if(j.version!==process.argv[1])process.exit(1);
    if(process.env.INTERNAL_API_SECRET && s.includes(process.env.INTERNAL_API_SECRET))process.exit(3);})" "'"$VERSION"'"
'
