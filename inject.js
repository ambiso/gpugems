const callback = (mutationList, observer) => {
  document.querySelectorAll(".copy-button").forEach((x) => x.remove());
  document
    .querySelectorAll('div[aria-label="Privacy"]')
    .forEach((x) => x.remove());
  document.querySelectorAll("#onetrust-consent-sdk").forEach((x) => x.remove());
};

const observer = new MutationObserver(callback);
observer.observe(document.documentElement, {
  attributes: true,
  childList: true,
  subtree: true,
});
