<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { FolderOpen, Trash2, Download, RotateCcw, Loader2 } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import DatabaseIcon from "@/components/icons/DatabaseIcon.vue";
import { useToast } from "@/composables/useToast";
import { isTauriRuntime } from "@/lib/tauriRuntime";
import type { JdbcDriverInfo, JdbcPluginStatus } from "@/types/database";
import * as api from "@/lib/api";

const { t } = useI18n();
const { toast } = useToast();
const isWeb = !isTauriRuntime();

// ──────────── Agent drivers ────────────

interface AgentDriverInfo {
  db_type: string;
  label: string;
  version: string;
  size: number;
  installed: boolean;
  installed_version: string | null;
  update_available: boolean;
}

interface InstallProgress {
  step: string;
  downloaded?: number;
  total?: number;
}

const drivers = ref<AgentDriverInfo[]>([]);
const jreInstalled = ref(false);
const installing = ref<string | null>(null);
const reinstallingJre = ref(false);
const progress = ref<InstallProgress | null>(null);

let unlisten: UnlistenFn | null = null;

const progressText = computed(() => {
  const p = progress.value;
  if (!p) return "";
  if (p.step === "jre-extract") return "解压 JRE...";
  const label = p.step === "jre" ? "下载 JRE" : "下载驱动";
  if (!p.total) return `${label}...`;
  const pct = Math.round(((p.downloaded ?? 0) / p.total) * 100);
  const dl = formatSize(p.downloaded ?? 0);
  const total = formatSize(p.total);
  return `${label}  ${dl} / ${total}  (${pct}%)`;
});

const progressPercent = computed(() => {
  const p = progress.value;
  if (!p || !p.total) return 0;
  return Math.round(((p.downloaded ?? 0) / p.total) * 100);
});

async function refreshAgents() {
  jreInstalled.value = await invoke<boolean>("check_jre_installed");
  drivers.value = await invoke<AgentDriverInfo[]>("list_installed_agents");
}

async function installDriver(dbType: string) {
  const label = drivers.value.find((d) => d.db_type === dbType)?.label ?? dbType;
  installing.value = dbType;
  progress.value = null;
  try {
    await invoke("install_agent", { dbType });
    await refreshAgents();
    toast(`${label} 驱动安装成功`);
  } catch (e: any) {
    toast(`${label} 驱动安装失败: ${e}`);
  } finally {
    installing.value = null;
    progress.value = null;
  }
}

async function uninstallDriver(dbType: string) {
  const label = drivers.value.find((d) => d.db_type === dbType)?.label ?? dbType;
  try {
    await invoke("uninstall_agent", { dbType });
    await refreshAgents();
    toast(`${label} 驱动已卸载`);
  } catch (e: any) {
    toast(`${label} 驱动卸载失败: ${e}`);
  }
}

async function reinstallJre() {
  reinstallingJre.value = true;
  progress.value = null;
  try {
    await invoke("reinstall_jre");
    await refreshAgents();
    toast("JRE 重新安装成功");
  } catch (e: any) {
    toast(`JRE 重新安装失败: ${e}`);
  } finally {
    reinstallingJre.value = false;
    progress.value = null;
  }
}

function formatSize(bytes: number): string {
  if (!bytes) return "";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// ──────────── JDBC drivers ────────────

const jdbcDrivers = ref<JdbcDriverInfo[]>([]);
const isLoadingJdbcDrivers = ref(false);
const jdbcPluginStatus = ref<JdbcPluginStatus | null>(null);
const isInstallingJdbcPlugin = ref(false);
const isUninstallingJdbcPlugin = ref(false);
const jdbcDriverPathInput = ref("");

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

async function loadJdbcDrivers() {
  if (isWeb) return;
  isLoadingJdbcDrivers.value = true;
  try {
    jdbcDrivers.value = await api.listJdbcDrivers();
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  } finally {
    isLoadingJdbcDrivers.value = false;
  }
}

async function loadJdbcPluginStatus() {
  if (isWeb) return;
  try {
    jdbcPluginStatus.value = await api.jdbcPluginStatus();
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  }
}

async function installJdbcPlugin() {
  if (isWeb || isInstallingJdbcPlugin.value) return;
  isInstallingJdbcPlugin.value = true;
  try {
    jdbcPluginStatus.value = await api.installJdbcPlugin();
    toast(t("settings.jdbcPluginInstallSuccess"));
    await loadJdbcDrivers();
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  } finally {
    isInstallingJdbcPlugin.value = false;
  }
}

async function uninstallJdbcPlugin() {
  if (isWeb || isUninstallingJdbcPlugin.value) return;
  isUninstallingJdbcPlugin.value = true;
  try {
    jdbcPluginStatus.value = await api.uninstallJdbcPlugin();
    toast(t("settings.jdbcPluginUninstallSuccess"));
    await loadJdbcDrivers();
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  } finally {
    isUninstallingJdbcPlugin.value = false;
  }
}

async function importJdbcDriverPaths(paths: string[]) {
  if (!paths.length) return;
  try {
    jdbcDrivers.value = await api.importJdbcDrivers(paths);
    jdbcDriverPathInput.value = "";
    toast(t("settings.jdbcImportSuccess", { count: paths.length }));
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  }
}

async function importJdbcDrivers() {
  if (isWeb) return;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    title: t("settings.jdbcImport"),
    multiple: true,
    filters: [{ name: "JDBC Driver", extensions: ["jar"] }],
  });
  if (!selected) return;

  const paths = (Array.isArray(selected) ? selected : [selected]).filter(
    (path): path is string => typeof path === "string",
  );
  await importJdbcDriverPaths(paths);
}

async function importJdbcDriverPathInput() {
  const paths = jdbcDriverPathInput.value
    .split(/\r?\n/)
    .map((path) => path.trim())
    .filter(Boolean);
  await importJdbcDriverPaths(paths);
}

async function deleteJdbcDriver(path: string) {
  try {
    jdbcDrivers.value = await api.deleteJdbcDriver(path);
    toast(t("settings.jdbcDeleteSuccess"));
  } catch (e: any) {
    toast(String(e?.message || e), 5000);
  }
}

// ──────────── Lifecycle ────────────

onMounted(async () => {
  await refreshAgents();
  unlisten = await listen<InstallProgress>("agent-install-progress", (event) => {
    if (event.payload.step === "done") {
      progress.value = null;
    } else {
      progress.value = event.payload;
    }
  });
  void loadJdbcDrivers();
  void loadJdbcPluginStatus();
});

onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <div class="h-full flex flex-col">
    <div class="flex-1 min-h-0 overflow-y-auto">
      <div class="max-w-4xl mx-auto px-6 py-6">
        <Tabs default-value="agent">
          <TabsList class="w-fit">
            <TabsTrigger value="agent">内置驱动</TabsTrigger>
            <TabsTrigger value="jdbc">JDBC 驱动</TabsTrigger>
          </TabsList>

          <!-- Agent Tab -->
          <TabsContent value="agent" class="mt-5 space-y-5">
            <!-- JRE Runtime -->
            <div class="rounded-xl border bg-muted/20 p-4">
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0">
                  <div class="text-sm font-medium">JRE 运行时</div>
                  <p v-if="!jreInstalled" class="text-xs text-muted-foreground mt-0.5">首次安装驱动时自动下载</p>
                </div>
                <div class="flex shrink-0 items-center gap-3">
                  <span v-if="jreInstalled" class="text-xs text-green-600">已安装</span>
                  <span v-else class="text-xs text-muted-foreground">未安装</span>
                  <Button
                    v-if="jreInstalled"
                    type="button"
                    variant="outline"
                    size="sm"
                    :disabled="reinstallingJre || installing !== null"
                    @click="reinstallJre"
                  >
                    <RotateCcw class="h-3.5 w-3.5 mr-1" />
                    {{ reinstallingJre ? "重装中..." : "重新安装" }}
                  </Button>
                </div>
              </div>
            </div>

            <!-- Progress bar -->
            <div v-if="progress" class="space-y-1.5 px-1">
              <div class="text-xs text-muted-foreground">{{ progressText }}</div>
              <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                <div
                  class="h-full rounded-full bg-primary transition-all duration-200"
                  :style="{ width: `${progressPercent}%` }"
                />
              </div>
            </div>

            <!-- Driver List -->
            <div v-if="drivers.length === 0" class="py-12 text-center text-sm text-muted-foreground">加载中...</div>
            <div v-else class="rounded-md border divide-y">
              <div
                v-for="driver in drivers"
                :key="driver.db_type"
                class="flex items-center gap-3 px-4 py-2.5 transition hover:bg-muted/30"
              >
                <span class="flex h-9 w-9 items-center justify-center rounded-lg bg-muted/60 shrink-0">
                  <DatabaseIcon :db-type="driver.db_type" class="h-5 w-5" />
                </span>
                <div class="min-w-0 flex-1">
                  <div class="text-sm font-medium">{{ driver.label }}</div>
                </div>
                <div class="text-xs text-muted-foreground text-right shrink-0">
                  <span v-if="driver.installed">v{{ driver.installed_version }}</span>
                  <span v-else-if="formatSize(driver.size)">{{ formatSize(driver.size) }}</span>
                </div>
                <div class="flex shrink-0 items-center gap-2">
                  <Button
                    v-if="!driver.installed"
                    size="sm"
                    class="h-7 text-xs"
                    :disabled="installing !== null"
                    @click="installDriver(driver.db_type)"
                  >
                    <Loader2 v-if="installing === driver.db_type" class="h-3 w-3 animate-spin mr-1" />
                    <Download v-else class="h-3 w-3 mr-1" />
                    {{ installing === driver.db_type ? "安装中..." : "安装" }}
                  </Button>
                  <template v-else>
                    <span class="text-xs text-green-600">已安装</span>
                    <Button
                      v-if="driver.update_available"
                      size="sm"
                      variant="outline"
                      class="h-7 text-xs"
                      :disabled="installing !== null"
                      @click="installDriver(driver.db_type)"
                    >
                      {{ installing === driver.db_type ? "更新中..." : "更新" }}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 text-xs text-muted-foreground hover:text-destructive"
                      @click="uninstallDriver(driver.db_type)"
                    >
                      卸载
                    </Button>
                  </template>
                </div>
              </div>
            </div>
          </TabsContent>

          <!-- JDBC Tab -->
          <TabsContent value="jdbc" class="mt-5 space-y-5">
            <!-- JDBC Plugin -->
            <div class="rounded-xl border bg-muted/20 p-4">
              <div class="flex min-h-12 items-center justify-between gap-3">
                <div class="min-w-0 space-y-1">
                  <Label>{{ t("settings.jdbcPlugin") }}</Label>
                  <p v-if="!jdbcPluginStatus?.installed" class="text-xs text-muted-foreground">
                    {{ t("settings.jdbcPluginNotInstalled") }}
                  </p>
                </div>
                <div class="flex shrink-0 items-center gap-3">
                  <span
                    v-if="jdbcPluginStatus?.installed"
                    class="text-xs"
                    :class="jdbcPluginStatus.compatible ? 'text-green-600' : 'text-destructive'"
                  >
                    {{
                      jdbcPluginStatus.compatible
                        ? t("settings.jdbcPluginInstalled", {
                            version: jdbcPluginStatus.version || "-",
                          })
                        : t("settings.jdbcPluginIncompatible")
                    }}
                  </span>
                  <Button
                    v-if="jdbcPluginStatus?.installed"
                    type="button"
                    variant="outline"
                    :disabled="isUninstallingJdbcPlugin"
                    @click="uninstallJdbcPlugin"
                  >
                    {{ isUninstallingJdbcPlugin ? t("common.loading") : t("settings.jdbcPluginUninstall") }}
                  </Button>
                  <Button
                    v-else
                    type="button"
                    variant="default"
                    :disabled="isInstallingJdbcPlugin"
                    @click="installJdbcPlugin"
                  >
                    {{ isInstallingJdbcPlugin ? t("common.loading") : t("settings.jdbcPluginInstall") }}
                  </Button>
                </div>
              </div>
            </div>

            <!-- JDBC Drivers -->
            <div class="space-y-3">
              <div class="space-y-1">
                <Label>{{ t("settings.jdbcDrivers") }}</Label>
              </div>
              <div class="flex items-center gap-2">
                <Input
                  v-model="jdbcDriverPathInput"
                  class="flex-1"
                  :placeholder="t('settings.jdbcDriverPathPlaceholder')"
                  @keydown.enter.prevent="importJdbcDriverPathInput"
                />
                <Button variant="outline" :disabled="!jdbcDriverPathInput.trim()" @click="importJdbcDriverPathInput">
                  {{ t("settings.jdbcImportPath") }}
                </Button>
                <Button class="shrink-0" @click="importJdbcDrivers">
                  <FolderOpen class="h-4 w-4" />
                  {{ t("settings.jdbcImport") }}
                </Button>
              </div>
            </div>

            <div class="rounded-md border">
              <div v-if="isLoadingJdbcDrivers" class="p-4 text-sm text-muted-foreground">
                {{ t("common.loading") }}
              </div>
              <div v-else-if="jdbcDrivers.length === 0" class="p-4 text-sm text-muted-foreground">
                {{ t("settings.jdbcNoDrivers") }}
              </div>
              <div v-else class="divide-y">
                <div v-for="driver in jdbcDrivers" :key="driver.path" class="flex items-center gap-3 p-3">
                  <div class="min-w-0 flex-1">
                    <div class="truncate text-sm font-medium">{{ driver.name }}</div>
                    <div class="truncate text-xs text-muted-foreground">{{ driver.path }}</div>
                  </div>
                  <div class="shrink-0 text-xs text-muted-foreground">{{ formatBytes(driver.size) }}</div>
                  <Button variant="ghost" size="icon" class="h-8 w-8 shrink-0" @click="deleteJdbcDriver(driver.path)">
                    <Trash2 class="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>
          </TabsContent>
        </Tabs>
      </div>
    </div>
  </div>
</template>
