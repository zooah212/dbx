import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import test from "node:test";

test("Redis connection dialog exposes standalone, sentinel, and cluster modes", () => {
  const source = readFileSync("apps/desktop/src/components/connection/ConnectionDialog.vue", "utf8");

  assert.match(source, /redis_connection_mode: "standalone"/);
  assert.match(source, /form\.redis_connection_mode === 'sentinel'/);
  assert.match(source, /form\.redis_connection_mode === 'cluster'/);
  assert.match(source, /t\("connection\.redisStandaloneMode"\)/);
  assert.match(source, /t\("connection\.redisSentinelMode"\)/);
  assert.match(source, /t\("connection\.redisClusterMode"\)/);
  assert.match(source, /v-model="form\.redis_sentinel_nodes"/);
  assert.match(source, /v-model="form\.redis_sentinel_master"/);
  assert.match(source, /v-model="form\.redis_sentinel_username"/);
  assert.match(source, /v-model="form\.redis_sentinel_password"/);
  assert.match(source, /v-model="form\.redis_sentinel_tls"/);
  assert.match(source, /v-model="form\.redis_cluster_nodes"/);
});

test("Redis sentinel submit config normalizes nodes and uses the first sentinel as endpoint", () => {
  const source = readFileSync("apps/desktop/src/components/connection/ConnectionDialog.vue", "utf8");

  assert.match(source, /normalizeRedisSentinelNodes/);
  assert.match(source, /firstRedisSentinelEndpoint/);
  assert.match(source, /config\.host = firstNode\.host/);
  assert.match(source, /config\.port = firstNode\.port/);
  assert.match(source, /config\.redis_sentinel_master = config\.redis_sentinel_master\?\.trim\(\) \|\| ""/);
  assert.match(source, /config\.redis_connection_mode = "standalone"/);
});

test("Redis cluster submit config normalizes nodes and uses the first cluster seed as endpoint", () => {
  const source = readFileSync("apps/desktop/src/components/connection/ConnectionDialog.vue", "utf8");

  assert.match(source, /normalizeRedisClusterNodes/);
  assert.match(source, /firstRedisClusterEndpoint/);
  assert.match(source, /config\.redis_cluster_nodes = normalizeRedisClusterNodes\(config\.redis_cluster_nodes \|\| ""\)/);
  assert.match(source, /config\.host = firstNode\.host/);
  assert.match(source, /config\.port = firstNode\.port/);
});

test("Redis sentinel and cluster fields are typed and localized", () => {
  const typesSource = readFileSync("apps/desktop/src/types/database.ts", "utf8");
  const zhSource = readFileSync("apps/desktop/src/i18n/locales/zh-CN.ts", "utf8");
  const enSource = readFileSync("apps/desktop/src/i18n/locales/en.ts", "utf8");

  assert.match(typesSource, /redis_connection_mode\?: "standalone" \| "sentinel" \| "cluster"/);
  assert.match(typesSource, /redis_sentinel_master\?: string/);
  assert.match(typesSource, /redis_sentinel_nodes\?: string/);
  assert.match(typesSource, /redis_sentinel_password\?: string/);
  assert.match(typesSource, /redis_cluster_nodes\?: string/);
  assert.match(zhSource, /redisSentinelMode: "哨兵"/);
  assert.match(zhSource, /redisClusterMode: "集群"/);
  assert.match(enSource, /redisSentinelMode: "Sentinel"/);
  assert.match(enSource, /redisClusterMode: "Cluster"/);
});

test("Tauri Redis connection commands route sentinel and cluster configs through the matching connector", () => {
  const source = readFileSync("src-tauri/src/commands/connection.rs", "utf8");

  assert.match(source, /config\.uses_redis_sentinel\(\)/);
  assert.match(source, /config\.uses_redis_cluster\(\)/);
  assert.match(source, /db_config\.uses_redis_sentinel\(\)/);
  assert.match(source, /db_config\.uses_redis_cluster\(\)/);
  assert.match(source, /db::redis_driver::connect_sentinel\(&config\)/);
  assert.match(source, /db::redis_driver::connect_sentinel\(&db_config\)/);
  assert.match(source, /db::redis_driver::connect_cluster\(&config\)/);
  assert.match(source, /db::redis_driver::connect_cluster\(&db_config\)/);
});
