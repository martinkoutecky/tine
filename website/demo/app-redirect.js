(function () {
try {
var p = location.protocol;
if ((p === "http:" || p === "https:") && !/[?&]static(=|&|$)/.test(location.search)) {
location.replace("app/" + location.hash);
}
} catch (_) {}
})();
