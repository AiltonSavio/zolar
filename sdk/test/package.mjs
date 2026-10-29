import assert from 'node:assert/strict';
import { test } from 'node:test';

test('the built package imports in Node and its HTTP client reads a catalog', async () => {
  const { MerchantClient, WalletClient, checkout } = await import('@zolar/sdk');
  assert.equal(typeof WalletClient, 'function');
  assert.equal(typeof checkout, 'function');
  const catalog = [{ item: { id: 'ga', name: 'Ticket', mint: 'Mint', amount: 1, price_zats: 100000 }, left: 2 }];
  const client = new MerchantClient('http://merchant', async (url) => {
    assert.equal(url, 'http://merchant/items');
    return Response.json(catalog);
  });
  assert.deepEqual(await client.items(), catalog);
});
