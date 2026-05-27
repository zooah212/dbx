import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import test from "node:test";

test("Redis browser exposes key/value search modes", () => {
  const source = readFileSync("apps/desktop/src/components/redis/RedisKeyBrowser.vue", "utf8");

  assert.match(source, /type RedisSearchMode = "key" \| "value"/);
  assert.match(source, /searchMode\s*=\s*ref<RedisSearchMode>\("key"\)/);
  assert.match(source, /redisScanValues/);
  assert.match(source, /redis\.searchByKey/);
  assert.match(source, /redis\.searchByValue/);
});

test("Redis key search input starts blank while scanning all keys internally", () => {
  const source = readFileSync("apps/desktop/src/components/redis/RedisKeyBrowser.vue", "utf8");

  assert.match(source, /searchPattern\s*=\s*ref\(""\)/);
  assert.match(source, /searchPattern\.value\.trim\(\) \|\| "\*"/);
  assert.doesNotMatch(source, /searchPattern\.value = "\*"/);
});

test("Redis command input is visually distinct from search", () => {
  const source = readFileSync("apps/desktop/src/components/redis/RedisKeyBrowser.vue", "utf8");

  assert.match(source, /data-redis-command-input/);
  assert.match(source, /t\("redis\.commandWelcome"\)/);
  assert.match(source, /{{ commandPrompt }}/);
  assert.match(source, /ref="commandTerminalRef"/);
  assert.match(source, /@submit\.prevent="executeCommand"/);
  assert.match(source, /@keydown\.enter\.prevent="executeCommand"/);
  assert.match(source, /caret-\[#d7ba7d\]/);
  assert.doesNotMatch(source, /redis\.commandPrefix/);
});

test("Redis value search streams incremental scan pages from the browser", () => {
  const browserSource = readFileSync("apps/desktop/src/components/redis/RedisKeyBrowser.vue", "utf8");
  const driverSource = readFileSync("crates/dbx-core/src/db/redis_driver.rs", "utf8");

  assert.match(browserSource, /async function streamValueSearch/);
  assert.match(browserSource, /async function fillInitialKeyBatch/);
  assert.match(browserSource, /searchRequestId/);
  assert.match(browserSource, /redis\.searchingValues/);
  assert.match(browserSource, /flatKeys\.value\.length < targetCount/);
  assert.match(browserSource, /await fillInitialKeyBatch\(requestId\)/);
  assert.doesNotMatch(driverSource, /while\s+result\.len\(\)\s*<\s*target_count/);
});
