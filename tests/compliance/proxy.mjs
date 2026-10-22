// Test-only proxy for blossom-compliance. It is not part of the shipped image (the Dockerfile
// copies only Cargo.toml, Cargo.lock, rust-toolchain.toml and src).
//
// The suite expects one ordinary Blossom server. The blob app has two ports, and its paid paths
// need the connector's payment statement. This joins them into one URL: the paid paths
// (PUT /upload, /mirror, /media) go to the write port with a stated payment added, as the
// connector would add it, and everything else goes to the public port.
//
// Usage: node proxy.mjs <listen-port> <write-url> <public-url>

import http from "node:http";

const [listenPort, writeUrl, publicUrl] = process.argv.slice(2);
if (!listenPort || !writeUrl || !publicUrl) {
  console.error("usage: node proxy.mjs <listen-port> <write-url> <public-url>");
  process.exit(2);
}

const PAID_PATHS = new Set(["/upload", "/mirror", "/media"]);
const PAYMENT = {
  "x-toon-payer": "0x00000000000000000000000000000000000c0de5",
  "x-toon-amount": "1000000",
  "x-toon-chain": "evm:31337",
};

const server = http.createServer((req, res) => {
  const path = new URL(req.url, "http://proxy.test").pathname;
  const paid = req.method === "PUT" && PAID_PATHS.has(path);
  const target = new URL(req.url, paid ? writeUrl : publicUrl);

  const headers = { ...req.headers, host: target.host };
  if (paid) {
    // The connector sets these itself; a client's own copies never count.
    Object.assign(headers, PAYMENT);
  }
  const upstream = http.request(
    target,
    { method: req.method, headers },
    (response) => {
      res.writeHead(response.statusCode, response.headers);
      response.pipe(res);
    },
  );
  upstream.on("error", (error) => {
    res.writeHead(502, { "content-type": "text/plain" });
    res.end(`proxy: ${error.message}`);
  });
  req.pipe(upstream);
});

server.listen(Number(listenPort), "127.0.0.1", () => {
  console.log(`compliance proxy listening on ${listenPort}`);
});
