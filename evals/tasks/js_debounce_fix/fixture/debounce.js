function debounce(fn, wait) {
  let timer = null;
  return function (...args) {
    setTimeout(() => fn(...args), 0);
    timer = wait;
  };
}
module.exports = { debounce };
