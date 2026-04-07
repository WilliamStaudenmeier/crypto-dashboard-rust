const formatMoney = (value) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: value > 100 ? 0 : 2,
  }).format(value || 0);

const formatPercent = (value) => `${(value || 0).toFixed(2)}%`;
const EXPLICIT_API_BASE = (window.__API_BASE_URL__ || "").replace(/\/$/, "");
const DEFAULT_RENDER_API_BASE = "https://crypto-dashboard-cpp.onrender.com";
const API_BASE_URL =
  EXPLICIT_API_BASE ||
  (window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1" ? "" : DEFAULT_RENDER_API_BASE);
let currentMarkets = [];
let marketAnimationInterval = null;
let trendingAnimationInterval = null;
let trendingFlashTimeout = null;
let bootstrapRefreshInterval = null;
let isBootstrapRefreshInFlight = false;

const TRENDING_ANIMATION_INTERVAL_MS = 20000;
const TRENDING_HIGHLIGHT_DURATION_MS = 1000;
const BOOTSTRAP_REFRESH_INTERVAL_MS = 20000;

function apiUrl(path) {
  return `${API_BASE_URL}${path}`;
}

function refreshUrl() {
  const isLocal = window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1";
  return isLocal ? apiUrl("/api/bootstrap?refresh=1") : "/api/bootstrap-refresh";
}

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Request failed: ${url}`);
  }
  return response.json();
}

function refreshUrlWithCacheBust() {
  const separator = refreshUrl().includes("?") ? "&" : "?";
  return `${refreshUrl()}${separator}ts=${Date.now()}`;
}

function hasCompleteBootstrapData(payload) {
  return Boolean(payload?.global && payload?.trending && Array.isArray(payload?.markets));
}

function renderBootstrapPayload(payload) {
  renderGlobal(payload?.global);
  renderTrending(payload?.trending);
  currentMarkets = (payload?.markets || []).map((coin) => ({ ...coin }));
  renderMarkets(currentMarkets);
}

function renderGlobal(globalData) {
  const usd = globalData?.data?.total_market_cap?.usd || 0;
  const volume = globalData?.data?.total_volume?.usd || 0;
  const btcDom = globalData?.data?.market_cap_percentage?.btc || 0;
  const ethDom = globalData?.data?.market_cap_percentage?.eth || 0;

  document.getElementById("market-cap").textContent = formatMoney(usd);
  document.getElementById("volume").textContent = formatMoney(volume);
  document.getElementById("btc-dom").textContent = formatPercent(btcDom);
  document.getElementById("eth-dom").textContent = formatPercent(ethDom);
}

function renderTrending(payload) {
  const list = document.getElementById("trending-list");
  list.innerHTML = "";
  const coins = payload?.coins || [];
  coins.slice(0, 7).forEach(({ item }) => {
    const li = document.createElement("li");
    li.innerHTML = `<span>${item.name} (${item.symbol})</span><span>#${item.market_cap_rank || "-"}</span>`;
    list.appendChild(li);
  });
}

function renderMarkets(markets) {
  const body = document.getElementById("coins-body");
  body.innerHTML = "";

  markets.forEach((coin) => {
    const tr = document.createElement("tr");
    const change = coin.price_change_percentage_24h || 0;
    const changeClass = change >= 0 ? "change-up" : "change-down";

    tr.innerHTML = `
      <td>${coin.name} (${coin.symbol.toUpperCase()})</td>
      <td>${formatMoney(coin.current_price)}</td>
      <td class="${changeClass}">${formatPercent(change)}</td>
      <td>${formatMoney(coin.market_cap)}</td>
      <td>${coin.market_cap_rank || "-"}</td>
    `;

    body.appendChild(tr);
  });
}

function randint(min, max) {
  return Math.floor(Math.random() * (max - min + 1)) + min;
}

function animateMarketPrices() {
  if (!currentMarkets.length) {
    return;
  }

  currentMarkets = currentMarkets.map((coin) => {
    const step = randint(0, 1) * randint(-1, 1);
    const nextPrice = Math.max(0, (coin.current_price || 0) + step);
    return { ...coin, current_price: nextPrice };
  });

  renderMarkets(currentMarkets);
}

function startMarketAnimation() {
  if (marketAnimationInterval) {
    clearInterval(marketAnimationInterval);
  }

  marketAnimationInterval = setInterval(() => {
    animateMarketPrices();
  }, 10000);
}

function shuffleArray(values) {
  const copy = [...values];
  for (let i = copy.length - 1; i > 0; i -= 1) {
    const j = Math.floor(Math.random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy;
}

function animateTrendingTokens() {
  const list = document.getElementById("trending-list");
  if (!list) {
    return;
  }

  const rows = Array.from(list.querySelectorAll("li"));
  if (!rows.length) {
    return;
  }

  rows.forEach((row) => row.classList.remove("trending-highlight-row"));
  const shuffledRows = shuffleArray(rows);
  shuffledRows.forEach((row) => list.appendChild(row));
  const updatedRows = Array.from(list.querySelectorAll("li"));
  if (!updatedRows.length) {
    return;
  }

  const highlightIndex = Math.floor(Math.random() * updatedRows.length);
  const highlightedRow = updatedRows[highlightIndex];
  highlightedRow.classList.add("trending-highlight-row");

  if (trendingFlashTimeout) {
    clearTimeout(trendingFlashTimeout);
  }

  trendingFlashTimeout = setTimeout(() => {
    highlightedRow.classList.remove("trending-highlight-row");
    trendingFlashTimeout = null;
  }, TRENDING_HIGHLIGHT_DURATION_MS);
}

function startTrendingAnimation() {
  if (trendingAnimationInterval) {
    clearInterval(trendingAnimationInterval);
  }

  if (trendingFlashTimeout) {
    clearTimeout(trendingFlashTimeout);
    trendingFlashTimeout = null;
  }

  trendingAnimationInterval = setInterval(() => {
    animateTrendingTokens();
  }, TRENDING_ANIMATION_INTERVAL_MS);
}

async function refreshBootstrapData() {
  if (isBootstrapRefreshInFlight) {
    return;
  }

  isBootstrapRefreshInFlight = true;
  try {
    const refreshedData = await fetchJson(refreshUrlWithCacheBust());
    if (!hasCompleteBootstrapData(refreshedData)) {
      throw new Error("Refresh payload missing required sections");
    }
    renderBootstrapPayload(refreshedData);
  } catch (error) {
    console.error(error);
  } finally {
    isBootstrapRefreshInFlight = false;
  }
}

function startBootstrapRefreshLoop() {
  if (bootstrapRefreshInterval) {
    clearInterval(bootstrapRefreshInterval);
  }

  bootstrapRefreshInterval = setInterval(() => {
    refreshBootstrapData();
  }, BOOTSTRAP_REFRESH_INTERVAL_MS);
}


async function bootstrap() {
  try {
    let staleData = null;

    try {
      staleData = await fetchJson("/db.json");
      if (hasCompleteBootstrapData(staleData)) {
        // Show previous snapshot while waiting for a full live refresh payload.
        renderBootstrapPayload(staleData);
        startMarketAnimation();
        startTrendingAnimation();
      }
    } catch {
      staleData = null;
    }

    // Replace UI only after the full refreshed payload has completed.
    await refreshBootstrapData();
    startMarketAnimation();
    startTrendingAnimation();
    startBootstrapRefreshLoop();
  } catch (error) {
    console.error(error);
  }
}

bootstrap();
