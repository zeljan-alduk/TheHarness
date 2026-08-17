set -e
out=$(python3 check_json.py good.json); code=$?
[ "$code" = "0" ] && [ "$out" = "ok" ] || { echo "good.json: code=$code out=$out"; exit 1; }
set +e
python3 check_json.py bad.json >/dev/null 2>&1; [ $? = 1 ] || { echo "bad.json should exit 1"; exit 1; }
python3 check_json.py missing.json >/dev/null 2>&1; [ $? = 2 ] || { echo "missing file should exit 2"; exit 1; }
python3 check_json.py >/dev/null 2>&1; [ $? = 64 ] || { echo "no args should exit 64"; exit 1; }
echo ok
