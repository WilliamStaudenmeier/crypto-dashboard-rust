const fs = require("node:fs/promises");
const path = require("node:path");

const COINGECKO_BASE = process.env.COINGECKO_BASE_URL || "https://api.coingecko.com/api/v3";
const COINGECKO_API_KEY = process.env.COINGECKO_API_KEY || "";

function nowEpochMs() {
  return Date.now();
}

function coingeckoHeaders() {
  const headers = { Accept: "application/json" };
  if (COINGECKO_API_KEY) {
    headers["x-cg-pro-api-key"] = COINGECKO_API_KEY;
  }
  return headers;
}

async function fetchCoinGeckoJson(relativePath) {
  const response = await fetch(`${COINGECKO_BASE}${relativePath}`, {
    headers: coingeckoHeaders(),
  });

  if (!response.ok) {
    throw new Error(`CoinGecko fetch failed: ${relativePath} (${response.status})`);
  }

  return response.json();
}

async function fetchLiveBootstrapPayload() {
  const marketsPath =
    "/coins/markets?vs_currency=usd&order=market_cap_desc&sparkline=false&price_change_percentage=24h&per_page=20&page=1";

  const [global, trending, markets] = await Promise.all([
    fetchCoinGeckoJson("/global"),
    fetchCoinGeckoJson("/search/trending"),
    fetchCoinGeckoJson(marketsPath),
  ]);

  return {
    global,
    trending,
    markets,
    meta: {
      source: "vercel-live",
      updated_at_epoch_ms: nowEpochMs(),
    },
  };
}

async function readStaleSnapshot() {
  try {
    const snapshotPath = path.join(process.cwd(), "db.json");
    const raw = await fs.readFile(snapshotPath, "utf8");
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

module.exports = async (req, res) => {
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.setHeader("Cache-Control", "no-store");

  try {
    const payload = await fetchLiveBootstrapPayload();
    res.statusCode = 200;
    res.end(JSON.stringify(payload));
    return;
  } catch {
    const stale = await readStaleSnapshot();
    if (stale) {
      if (!stale.meta || typeof stale.meta !== "object") {
        stale.meta = {};
      }
      stale.meta.source = "vercel-snapshot-fallback";
      res.statusCode = 200;
      res.end(JSON.stringify(stale));
      return;
    }

    res.statusCode = 502;
    res.end(JSON.stringify({ error: "Failed to refresh bootstrap data" }));
  }
};
