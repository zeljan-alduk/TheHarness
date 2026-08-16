const prices = [10, 20, 30];

function fetchPrice(p) {
  return new Promise((resolve) => setTimeout(() => resolve(p), 10));
}

async function main() {
  let total = 0;
  prices.forEach(async (p) => {
    total += await fetchPrice(p);
  });
  console.log(`total: ${total}`);
}

main();
