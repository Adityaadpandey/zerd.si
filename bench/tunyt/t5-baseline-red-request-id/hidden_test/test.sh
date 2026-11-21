LIB="$(cd "$(dirname "$0")/../.." && pwd)/_lib"
sh "$LIB/with_server.sh" sh -c '
  h=$(curl -s -D - -o /dev/null "$API/health" ; curl -s -D - -o /dev/null "$API/")
  printf "%s" "$h" | grep -qi "^x-request-id: .\+" || { echo "no X-Request-Id on responses"; exit 1; }
  echo=$(curl -s -D - -o /dev/null -H "X-Request-Id: bench-fixed-id-123" "$API/" | grep -i "^x-request-id:" | tr -d "\r")
  printf "%s" "$echo" | grep -q "bench-fixed-id-123" || { echo "incoming X-Request-Id not reused: $echo"; exit 1; }
'
