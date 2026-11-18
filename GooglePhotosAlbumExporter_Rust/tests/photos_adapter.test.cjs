'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const adapter = fs.readFileSync(path.join(__dirname, '..', 'src', 'photos.js'), 'utf8');

function fixture(respond) {
  let address = 'expected@example.com';
  const requests = [];
  const context = {
    URL, URLSearchParams, Response, AbortSignal, TextDecoder,
    location: { origin: 'https://photos.google.com', pathname: '/u/2/photos' },
    window: { WIZ_global_data: {
      eptZe: '/u/2/_/PhotosUi/', FdrFJe: 'fixture-session',
      cfb2h: 'fixture-build', SNlM0e: 'fixture-request-value',
    } },
    document: {
      querySelectorAll: (selector) => selector.startsWith('a[href*="/album/"]') ? [] : [{
        checkVisibility: () => true, getAttribute: () => 'Google Account (' + address + ')',
      }],
    },
    fetch: async (url, options) => {
      const endpoint = new URL(url);
      assert.equal(endpoint.origin, 'https://photos.google.com');
      assert.equal(endpoint.pathname, '/u/2/_/PhotosUi/data/batchexecute');
      assert.equal(options.method, 'POST');
      assert.equal(options.redirect, 'manual');
      const envelope = JSON.parse(options.body.get('f.req'))[0][0];
      assert.equal(endpoint.searchParams.get('rpcids'), envelope[0]);
      const req = { rpc: envelope[0], params: JSON.parse(envelope[1]) };
      requests.push(req);
      return respond(req, (changed) => { address = changed; });
    },
  };
  return {
    requests, context, setAccount: value => { address = value; },
    run: async (...args) => JSON.parse(JSON.stringify(await vm.runInNewContext(adapter, context)('expected@example.com')(...args))),
    instance: () => vm.runInNewContext(adapter, context)('expected@example.com'),
  };
}

const value = (object) => JSON.parse(JSON.stringify(object));
const response = (rpc, data) => new Response(")]}'\n123\n" + JSON.stringify([
  ['wrb.fr', rpc, JSON.stringify(data), null, null, null, 'generic'],
], null, 2) + '\n');

test('uses the allowlisted list, member and bulk RPCs, and confines credentials to the page', async () => {
  const mock = fixture(({ rpc, params }) => {
    if (rpc === 'Z5xsfc') {
      const detail = [null, 'Shared title', null, 1, true, 'fixture-share-key'];
      return response(rpc, [[['album1', null, {72930366: detail}]], params[0] ? null : 'list-next']);
    }
    if (rpc === 'snAcKc') {
      assert.deepEqual(params, ['album1', null, null, 'fixture-share-key']);
      const info = Array(22).fill(null);
      info[0] = 'album1';
      info[21] = 1;
      return response(rpc, [null, [['media1', null, 1700000000123, 'dedup']], null, info]);
    }
    assert.equal(rpc, 'EWgK9e');
    assert.deepEqual(params[0][0], [[['media1']]]);
    assert.deepEqual(params[0][1][0][24], []);
    const item = Array(10).fill(null);
    item[3] = 'file.jpg';
    item[6] = 1700000000123;
    item[9] = 1234;
    return response(rpc, [['media1', item]]);
  });
  const run = mock.instance();
  assert.deepEqual(value(await run('verify', null)), {data: true});
  const first = value(await run('albums', null));
  assert.equal(first.data.next, 'list-next');
  assert.equal(first.data.items[0].is_shared, true);
  assert.equal(first.data.total, null);
  assert.doesNotMatch(JSON.stringify(first), /fixture-(share|request|session)/);
  await run('albums', 'list-next');
  const members = value(await run('members', {id: 'album1', next: null}));
  assert.equal(members.data.items[0].taken_ms, 1700000000123);
  assert.equal(members.data.reported_count, 1);
  const details = value(await run('metadata', ['media1']));
  assert.equal(details.data[0].file_name, 'file.jpg');
  assert.equal(details.data[0].size_bytes, 1234);
  assert.deepEqual(value(await run('write', {})), {error: 'UNSUPPORTED_OPERATION'});
  assert.deepEqual(mock.requests.map(req => req.rpc), ['Z5xsfc','Z5xsfc','snAcKc','EWgK9e']);
});

test('rejects wrong account, sign-in redirect, and account switch during fetch', async () => {
  const wrong = fixture(() => { throw new Error('fetch must not run'); });
  wrong.setAccount('other@example.com');
  assert.deepEqual(await wrong.run('albums', null), {error: 'ACCOUNT_MISMATCH'});
  assert.equal(wrong.requests.length, 0);
  wrong.context.location.origin = 'https://accounts.google.com';
  assert.deepEqual(await wrong.run('verify', null), {error: 'LOGIN_REQUIRED'});
  const switched = fixture(({rpc}, change) => { change('other@example.com'); return response(rpc, [[], null]); });
  assert.deepEqual(await switched.run('albums', null), {error: 'ACCOUNT_MISMATCH'});
});

test('distinguishes schema drift, a transient response, and denied album access', async () => {
  const bad = fixture(() => new Response('<html>unexpected</html>'));
  assert.deepEqual(await bad.run('albums', null), {error: 'API_SCHEMA', detail: 'RESPONSE_PREFIX'});
  const busy = fixture(() => new Response('', {status: 503}));
  assert.deepEqual(await busy.run('albums', null), {error: 'TEMPORARY_HTTP'});
  for (const options of [{status: 429}, {status: 503, headers: {'Retry-After': '120'}}]) {
    const limited = fixture(() => new Response('', options));
    assert.deepEqual(await limited.run('albums', null), {error: 'SERVER_WAIT'});
  }
  const denied = fixture(() => new Response('', {status: 403}));
  assert.deepEqual(await denied.run('members', {id: 'album1', next: null}), {error: 'ALBUM_ACCESS_DENIED'});
});

test('accepts an empty album only with explicit matching zero-count details', async () => {
  const blank = fixture(({rpc}) => {
    const details = Array(22).fill(null);
    details[0] = 'empty';
    details[21] = 0;
    return response(rpc, [null, null, null, details]);
  });
  const result = await blank.run('members', {id: 'empty', next: null});
  assert.deepEqual(result.data.items, []);
  assert.equal(result.data.reported_count, 0);
  assert.deepEqual(await blank.run('members', {id: 'other', next: null}), {error: 'API_SCHEMA', detail: 'ALBUM_ID'});
});

test('omits unidentified rows and handles the documented read-only single-item fallback', async () => {
  const mock = fixture(({rpc, params}) => {
    if (rpc === 'EWgK9e') return response(rpc, [[null, [null, null, null, 'must-not-assign.jpg']]]);
    assert.equal(rpc, 'fDcn4b');
    assert.deepEqual(params, ['media1', 1, null, null, 1]);
    return response(rpc, [['media1', null, 'confirmed.jpg', 1700000000123, null, 123]]);
  });
  const run = mock.instance();
  assert.deepEqual(value(await run('metadata', ['media1'])), {data: []});
  const single = value(await run('metadata_single', {album: 'album1', id: 'media1'}));
  assert.equal(single.data.file_name, 'confirmed.jpg');
  assert.deepEqual(mock.requests.map(req => req.rpc), ['EWgK9e', 'fDcn4b']);

  const absent = fixture(({rpc}) => new Response(")]}'\n" + JSON.stringify([['wrb.fr', rpc, null]])));
  assert.deepEqual(await absent.run('metadata_single', {album:'album1', id:'media1'}), {data:null});
  assert.deepEqual(await absent.run('albums', null), {error:'API_SCHEMA', detail:'RPC_PAYLOAD'});
});
