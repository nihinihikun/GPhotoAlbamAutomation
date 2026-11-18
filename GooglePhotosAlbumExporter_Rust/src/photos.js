/* Read-only field maps adapted from Google Photos Toolkit v3.3.0.
 * Copyright (c) 2024 xob0t. MIT license: docs/TOOLKIT_LICENSE.txt.
 * Account and request credentials stay inside this page's CDP object.
 */
(function (expectedAccount) {
  "use strict";

  const origin = "https://photos.google.com";
  const shareKeys = new Map();
  let verifiedSlot = null;
  let verifiedSession = null;
  const selectors = [
    'a[href^="https://accounts.google.com/SignOutOptions"][aria-label]',
    'header a[aria-label], header button[aria-label]',
    '[role="banner"] a[aria-label], [role="banner"] button[aria-label]',
    'button[aria-label^="Google Account"], button[aria-label^="Google アカウント"]',
  ].join(",");
  const emailPattern = /[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi;

  function account() {
    if (location.origin !== origin) return "LOGIN_REQUIRED";
    const addresses = new Set();
    for (const node of document.querySelectorAll(selectors)) {
      const label = node.getAttribute("aria-label") || "";
      if (!node.checkVisibility({ opacityProperty: true, visibilityProperty: true }) ||
          !/(account|アカウント|cuenta|konto|compte|conta|帳戶|계정)/i.test(label)) continue;
      for (const address of label.match(emailPattern) || []) addresses.add(address.toLowerCase());
    }
    if (addresses.size !== 1) return "ACCOUNT_UNVERIFIED";
    if (!addresses.has(expectedAccount)) return "ACCOUNT_MISMATCH";
    const wiz = window.WIZ_global_data;
    const widgetAccount = wiz?.oPEP7c;
    if (typeof widgetAccount === "string" &&
        widgetAccount.match(emailPattern)?.includes(widgetAccount) &&
        widgetAccount.toLowerCase() !== expectedAccount) return "ACCOUNT_MISMATCH";
    const pageSlot = location.pathname.match(/^\/u\/(\d+)(?:\/|$)/)?.[1] || "0";
    const rpcRoute = typeof wiz?.eptZe === "string"
      ? wiz.eptZe.match(/^\/(?:u\/(\d+)\/)?_\/PhotosUi\/$/)
      : null;
    if (!rpcRoute || (rpcRoute[1] || "0") !== pageSlot ||
        ![wiz.FdrFJe, wiz.cfb2h, wiz.SNlM0e].every(value => typeof value === "string" && value)) {
      return "ACCOUNT_UNVERIFIED";
    }
    if (verifiedSlot === null) {
      verifiedSlot = pageSlot;
      verifiedSession = wiz.FdrFJe;
    }
    if (pageSlot !== verifiedSlot || wiz.FdrFJe !== verifiedSession) return "ACCOUNT_CHANGED";
    return null;
  }

  function fail(code, detail) {
    return detail ? { error: code, detail } : { error: code };
  }

  class SchemaError extends Error {}

  function expectArray(value, field = "ARRAY") {
    if (!Array.isArray(value)) throw new SchemaError(field);
    return value;
  }

  function optional(value, predicate, field) {
    if (value == null) return null;
    if (!predicate(value)) throw new SchemaError(field);
    return value;
  }

  const string = (value, limit = 4096, field = "TEXT") => optional(value, v => typeof v === "string" && v.length <= limit, field);
  const count = (value, field = "COUNT") => optional(value, v => Number.isSafeInteger(v) && v >= 0, field);
  const timestamp = value => optional(value, Number.isSafeInteger, "TIMESTAMP");
  const cursor = value => string(value, 16384, "CURSOR") || null;

  function key(value) {
    const result = string(value, 512, "ID");
    if (!result || !/^[^\x00-\x1f\x7f]+$/.test(result)) throw new SchemaError("ID");
    return result;
  }

  function parseResponse(text, rpc) {
    if (!text.startsWith(")]}'")) throw new SchemaError("RESPONSE_PREFIX");
    let pending = "";
    let payload;
    for (const line of text.slice(4).split(/\r?\n/)) {
      if (!pending && (/^\s*$/.test(line) || /^\d+\s*$/.test(line))) continue;
      pending += line + "\n";
      let frame;
      try { frame = JSON.parse(pending); } catch { continue; }
      pending = "";
      for (const entry of expectArray(frame, "RESPONSE_FRAME")) {
        if (entry?.[0] !== "wrb.fr" || entry[1] !== rpc) continue;
        if (payload !== undefined) throw new SchemaError("RPC_PAYLOAD");
        if (entry[2] === null && (rpc === "EWgK9e" || rpc === "fDcn4b")) {
          payload = [];
        } else {
          if (typeof entry[2] !== "string") throw new SchemaError("RPC_PAYLOAD");
          payload = expectArray(JSON.parse(entry[2]), "RPC_PAYLOAD");
        }
      }
    }
    if (pending.trim()) throw new SchemaError("RESPONSE_FRAME");
    if (payload === undefined) throw new SchemaError("RPC_MISSING");
    return payload;
  }

  async function readBody(response) {
    const reader = response.body?.getReader();
    if (!reader) throw new Error();
    let size = 0;
    let text = "";
    const decoder = new TextDecoder("utf-8", { fatal: true });
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) return text + decoder.decode();
        size += value.byteLength;
        if (size > 8 * 1024 * 1024) throw new Error();
        text += decoder.decode(value, { stream: true });
      }
    } finally {
      await reader.cancel().catch(() => {});
    }
  }

  async function rpcRequest(rpc, args) {
    const wiz = window.WIZ_global_data;
    const params = new URLSearchParams({
      rpcids: rpc,
      "source-path": location.pathname,
      "f.sid": wiz.FdrFJe,
      bl: wiz.cfb2h,
      pageId: "none",
      rt: "c",
    });
    let response;
    try {
      response = await fetch(origin + wiz.eptZe + "data/batchexecute?" + params, {
        method: "POST",
        credentials: "same-origin",
        redirect: "manual",
        headers: { "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8" },
        body: new URLSearchParams({
          "f.req": JSON.stringify([[[rpc, JSON.stringify(args), null, "generic"]]]),
          at: wiz.SNlM0e,
        }),
        signal: AbortSignal.timeout(15000),
      });
    } catch {
      return fail("TEMPORARY_NETWORK");
    }
    const responseAccount = account();
    if (responseAccount) return fail(responseAccount);
    if (response.status === 403 && rpc === "snAcKc") return fail("ALBUM_ACCESS_DENIED");
    if (response.status === 403 && (rpc === "EWgK9e" || rpc === "fDcn4b")) return fail("METADATA_ACCESS_DENIED");
    if (response.type === "opaqueredirect" || response.status === 401 || response.status === 403) {
      return fail("LOGIN_OR_ACCESS_REQUIRED");
    }
    if (response.status === 429 ||
        (response.status === 503 && response.headers.has("Retry-After"))) {
      return fail("SERVER_WAIT");
    }
    if (response.status === 408 || response.status >= 500) {
      return fail("TEMPORARY_HTTP");
    }
    if (!response.ok) return fail("HTTP_ERROR");
    let body;
    try { body = await readBody(response); }
    catch (error) {
      return fail(error.name === "TimeoutError" || error.name === "AbortError" || error instanceof TypeError
        ? "TEMPORARY_NETWORK" : "API_SCHEMA");
    }
    const status = account();
    if (status) return fail(status);
    try { return { payload: parseResponse(body, rpc) }; }
    catch (error) { return fail("API_SCHEMA", error instanceof SchemaError ? error.message : "RPC_PAYLOAD"); }
  }

  function albumPage(payload) {
    if (payload.length < 2) throw new SchemaError("ALBUM_LIST");
    const next = cursor(payload[1]);
    const rows = payload[0] == null && !next ? [] : expectArray(payload[0], "ALBUM_LIST");
    const urls = new Map();
    for (const anchor of document.querySelectorAll('a[href*="/album/"]')) {
      const url = new URL(anchor.href, origin);
      const match = url.pathname.match(/^\/(?:u\/(\d+)\/)?album\/([^/]+)$/);
      if (url.origin === origin && !url.search && !url.hash && match &&
          (match[1] || "0") === verifiedSlot) {
        try { urls.set(decodeURIComponent(match[2]), url.href); } catch { /* Invalid URL in the page. */ }
      }
    }
    const items = rows.map(row => {
      const id = key(expectArray(row)[0]);
      const details = row.at(-1)?.[72930366];
      if (details != null) expectArray(details, "ALBUM_ATTRIBUTES");
      const auth = string(details?.[5], 4096, "ALBUM_SHARE_KEY");
      if (auth) shareKeys.set(id, auth);
      const shared = optional(details?.[4], v => v === true || v === false || v === 0 || v === 1, "ALBUM_SHARED_FLAG");
      return {
        id, title: string(details?.[1], 4096, "ALBUM_TITLE"), url: urls.get(id) ?? null,
        is_shared: shared === null ? null : Boolean(shared),
        owner_actor_id: string(row[6]?.[0], 512, "ALBUM_OWNER"), reported_count: count(details?.[3], "ALBUM_LIST_COUNT"),
      };
    });
    return { items, next, total: null };
  }

  function memberPage(payload, id) {
    if (payload.length < 3) throw new SchemaError("ALBUM_MEMBERS");
    const details = payload[3];
    if (details != null) expectArray(details, "ALBUM_ATTRIBUTES");
    const albumId = details?.[0] == null ? null : key(details[0]);
    if (albumId !== null && albumId !== id) throw new SchemaError("ALBUM_ID");
    const reported = count(details?.[21], "ALBUM_MEMBER_COUNT");
    const next = cursor(payload[2]);
    const rows = payload[1] == null && !next && albumId === id && reported === 0
      ? [] : expectArray(payload[1], "ALBUM_MEMBERS");
    return {
      id: albumId, reported_count: reported, next,
      items: rows.map(row => ({
        id: key(expectArray(row)[0]),
        taken_ms: timestamp(row[2]), creation_ms: timestamp(row[5]),
        dedup_key: string(row[3]),
      })),
    };
  }

  function mediaInfo(payload, requestSize) {
    if (payload.length > requestSize) throw new SchemaError("METADATA_ROW_COUNT");
    const items = [];
    for (const row of payload) {
      expectArray(row, "METADATA_ROW");
      if (row[1] != null) expectArray(row[1], "METADATA_ROW");
      // Some results have no media ID. Never match those rows by request position.
      if (row[0] == null) continue;
      items.push({
        id: key(row[0]), file_name: string(row[1]?.[3]), taken_ms: timestamp(row[1]?.[6]),
        creation_ms: timestamp(row[1]?.[8]), size_bytes: count(row[1]?.[9]),
      });
    }
    return items;
  }

  function singleMediaInfo(payload) {
    if (payload[0] == null) return null;
    const row = expectArray(payload[0], "METADATA_ROW");
    if (row[0] == null) return null;
    return {
      id: key(row[0]), file_name: string(row[2]), taken_ms: timestamp(row[3]),
      creation_ms: null, size_bytes: count(row[5]),
    };
  }

  // All RPC IDs, shapes and routes are fixed locally. No caller-supplied endpoint.
  return async function (op, argument) {
    const status = account();
    if (status) return fail(status);
    if (op === "verify") return { data: true };
    try {
      let response;
      switch (op) {
        case "albums":
          response = await rpcRequest("Z5xsfc", [cursor(argument), null, null, null, 1, null, null, 100, [2], 5]);
          return response.error ? response : { data: albumPage(response.payload) };
        case "members": {
          const id = key(argument.id);
          response = await rpcRequest("snAcKc", [id, cursor(argument.next), null, shareKeys.get(id) ?? null]);
          return response.error ? response : { data: memberPage(response.payload, id) };
        }
        case "metadata": {
          const keys = expectArray(argument);
          if (!keys.length || keys.length > 40) throw new Error();
          const fields = Array(36).fill(null);
          fields[24] = [];
          fields[35] = [];
          response = await rpcRequest("EWgK9e", [[[keys.map(value => [key(value)])], [fields]]]);
          if (response.error === "METADATA_ACCESS_DENIED") return { data: [] };
          return response.error ? response : { data: mediaInfo(response.payload, keys.length) };
        }
        case "metadata_single": {
          const id = key(argument.id);
          const album = key(argument.album);
          response = await rpcRequest("fDcn4b", [id, 1, shareKeys.get(album) ?? null, null, 1]);
          if (response.error === "METADATA_ACCESS_DENIED") return { data: null };
          return response.error ? response : { data: singleMediaInfo(response.payload) };
        }
        default:
          return fail("UNSUPPORTED_OPERATION");
      }
    } catch (error) {
      return fail("API_SCHEMA", error instanceof SchemaError ? error.message : "NORMALIZATION");
    }
  };
})
