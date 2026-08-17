set -e
out=$(sh summarize.sh access.log)
expected="200 3
404 2
500 1"
[ "$out" = "$expected" ] || { echo "got:"; echo "$out"; echo "want:"; echo "$expected"; exit 1; }
echo ok
