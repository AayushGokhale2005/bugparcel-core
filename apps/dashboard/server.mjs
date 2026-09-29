import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import os from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const coreRoot = resolve(here, "../..");
const parcelHome = process.env.BUGPARCEL_HOME ?? resolve(os.homedir(), "Documents/bugparcel-auto-captures");
const allowedTools = new Set([
  "bugparcel_list_parcels",
  "bugparcel_get_parcel",
  "bugparcel_reproduce",
  "bugparcel_diagnose",
]);

function invokeMcp(name, arguments_ = {}) {
  if (!allowedTools.has(name)) throw new Error("Dashboard cannot invoke that MCP tool.");
  return new Promise((resolvePromise, reject) => {
    const child = spawn("cargo", ["run", "--quiet", "--manifest-path", resolve(coreRoot, "Cargo.toml"), "-p", "bugparcel-mcp"], {
      cwd: coreRoot,
      env: { ...process.env, BUGPARCEL_HOME: parcelHome },
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) return reject(new Error(stderr || `MCP exited with ${code}`));
      try {
        const message = stdout.trim().split("\n").map(JSON.parse).find((value) => value.id === 1);
        if (message?.error) throw new Error(message.error.message);
        resolvePromise(message?.result?.structuredContent ?? message?.result);
      } catch (error) {
        reject(error);
      }
    });
    child.stdin.end(`${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: arguments_ } })}\n`);
  });
}

function respond(res, status, body) {
  res.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  res.end(JSON.stringify(body));
}

function readBody(req) {
  return new Promise((resolvePromise, reject) => {
    let text = "";
    req.on("data", (chunk) => { text += chunk; });
    req.on("end", () => { try { resolvePromise(text ? JSON.parse(text) : {}); } catch (error) { reject(error); } });
    req.on("error", reject);
  });
}

createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1:4318");
    if (req.method === "GET" && url.pathname === "/api/parcels") return respond(res, 200, await invokeMcp("bugparcel_list_parcels"));
    if (req.method === "GET" && url.pathname.startsWith("/api/parcels/")) {
      return respond(res, 200, await invokeMcp("bugparcel_get_parcel", { parcel_id: decodeURIComponent(url.pathname.split("/").at(-1)) }));
    }
    if (req.method === "POST" && url.pathname === "/api/actions") {
      const body = await readBody(req);
      return respond(res, 200, await invokeMcp(body.tool, { parcel_id: body.parcel_id }));
    }
    respond(res, 404, { error: "Not found" });
  } catch (error) {
    respond(res, 500, { error: error instanceof Error ? error.message : "Unexpected local bridge error" });
  }
}).listen(4318, "127.0.0.1", () => {
  console.log(`BugParcel local bridge: http://127.0.0.1:4318 · store ${parcelHome}`);
});
