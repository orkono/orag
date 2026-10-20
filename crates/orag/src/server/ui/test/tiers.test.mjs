// The answer model choice as view models (step 82): the machine card's rows,
// each tier's text and state, the start button's total, and the
// unsupported messages, in both languages.
import { test, afterEach } from "node:test";
import assert from "node:assert/strict";
import { setLanguage } from "../i18n.js";
import { choiceTotal, choiceView, machineRows } from "../tiers.js";

const GB = 1024 ** 3;

const mac = {
  os: "macos",
  arch: "aarch64",
  total_ram_bytes: 16 * GB,
  available_ram_bytes: 10 * GB,
  logical_cores: 12,
  physical_cores: 12,
  cpu_model: "Apple M4 Pro",
  cpu_features: "not_applicable",
  apple_silicon: true,
  metal: true,
  free_disk_bytes: 200 * GB,
  gpu: null,
};

const option = (id, size, bytes, need, verdict, extra = {}) => ({
  id,
  size,
  label_key: `setup.tier.${size}`,
  bytes,
  installed: false,
  need_ram_bytes: need,
  speed: { metal_p50_answer_s: null, cpu_p50_answer_s: null },
  verdict,
  warnings: [],
  ...extra,
});

function models({ machine = mac, options, selected, warnings = [], support } = {}) {
  return {
    mode: "setup",
    machine,
    download: {
      available: true,
      bytes: 3 * GB,
      embedding_bytes: 0.5 * GB,
      options,
      selected,
      warnings,
      support: support ?? { status: "supported", code: null, missing_cpu_features: [] },
    },
  };
}

const twoFit = () =>
  models({
    options: [
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "recommended", { speed: { metal_p50_answer_s: 0.9, cpu_p50_answer_s: null } }),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "fits", { speed: { metal_p50_answer_s: 2.5, cpu_p50_answer_s: 14 } }),
    ],
    selected: "qwen3.5-4b-q4_k_m",
  });

afterEach(() => setLanguage("tr"));

test("the card names this Mac's memory, processor, cores, GPU and disk", () => {
  const rows = machineRows(mac).map(({ label, value }) => `${label}: ${value}`);
  assert.deepEqual(rows, [
    "Bellek: 16 GB (şu an boş: 10 GB)",
    "İşlemci: Apple M4 Pro",
    "Çekirdek: 12",
    "Ekran kartı: Apple GPU (Metal)",
    "Boş disk: 200 GB",
  ]);
  setLanguage("en");
  assert.equal(machineRows(mac)[0].value, "16 GB (10 GB free now)");
});

test("a PC's card shows physical and logical cores, its GPU, and unknowns", () => {
  const pc = {
    ...mac,
    metal: false,
    apple_silicon: false,
    cpu_model: "Intel(R) Core(TM) i7-4770",
    logical_cores: 8,
    physical_cores: 4,
    available_ram_bytes: null,
    free_disk_bytes: null,
    gpu: { name: "NVIDIA GeForce RTX 3060", memory_bytes: 12 * GB },
  };
  const rows = machineRows(pc).map(({ label, value }) => `${label}: ${value}`);
  assert.deepEqual(rows, [
    "Bellek: 16 GB",
    "İşlemci: Intel(R) Core(TM) i7-4770",
    "Çekirdek: 4 fiziksel, 8 mantıksal",
    "Ekran kartı: NVIDIA GeForce RTX 3060 (12 GB)",
    "Boş disk: bilinmiyor",
  ]);
  assert.deepEqual(machineRows(null), []);
});

test("each tier shows its name, size, need, speed on this machine and verdict", () => {
  const view = choiceView(twoFit());
  assert.equal(view.selected, "qwen3.5-4b-q4_k_m");
  assert.deepEqual(
    view.options.map((o) => [o.name, o.badge, o.details, o.speed, o.eligible]),
    [
      ["Standart", "Önerilen", "2,5 GB indirme · 6,5 GB bellek gerekir", "Tipik yanıt süresi: Apple GPU'da yaklaşık 0,9 sn (tahmin)", true],
      ["Büyük: daha yavaş, daha çok bellek kullanır", "Uygun", "5,5 GB indirme · 9 GB bellek gerekir", "Tipik yanıt süresi: Apple GPU'da yaklaşık 2,5 sn (tahmin)", true],
    ],
  );
  assert.equal(view.total, 3 * GB, "the embedder plus the default tier");
  assert.equal(choiceTotal(twoFit().download, "qwen3.5-9b-q4_k_m"), 6 * GB);
  assert.equal(view.unsupported, null);
  assert.deepEqual(view.warnings, []);
  setLanguage("en");
  const en = choiceView(twoFit());
  assert.deepEqual(en.options.map((o) => o.badge), ["Recommended", "Fits"]);
  assert.equal(en.options[1].name, "Large: slower, uses more memory");
  assert.equal(en.options[0].details, "2.5 GB download · needs 6.5 GB of memory");
});

test("a pick stays while it is eligible; otherwise the server's selection", () => {
  assert.equal(choiceView(twoFit(), "qwen3.5-9b-q4_k_m").selected, "qwen3.5-9b-q4_k_m");
  assert.equal(choiceView(twoFit(), "gone").selected, "qwen3.5-4b-q4_k_m");
});

test("the configured generator that is not a tier is the current option", () => {
  const current = models({
    options: [
      option("qwen3-4b-instruct-2507-q4_k_m", null, 2.3 * GB, 6.5 * GB, "fits", { current: true, label_key: "setup.tier.current" }),
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "recommended"),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "fits"),
    ],
    selected: "qwen3-4b-instruct-2507-q4_k_m",
  });
  const view = choiceView(current);
  assert.equal(view.options[0].name, "Şu anki: qwen3-4b-instruct-2507-q4_k_m (config.toml'da ayarlı)");
  assert.equal(view.selected, "qwen3-4b-instruct-2507-q4_k_m");
  setLanguage("en");
  assert.equal(choiceView(current).options[0].name, "Current: qwen3-4b-instruct-2507-q4_k_m (set in config.toml)");
});

test("an old CPU outside Windows is a slow-CPU warning, not a block", () => {
  const slow = models({
    machine: { ...mac, metal: false, cpu_features: { avx2: false, fma: true, f16c: true, bmi2: true } },
    options: twoFit().download.options,
    selected: "qwen3.5-4b-q4_k_m",
    warnings: ["slow_cpu"],
    support: { status: "supported", code: null, missing_cpu_features: ["avx2"] },
  });
  const view = choiceView(slow);
  assert.equal(view.unsupported, null);
  assert.equal(view.selected, "qwen3.5-4b-q4_k_m");
  assert.equal(view.warnings.length, 1);
  assert.match(view.warnings[0], /\(AVX2\).*daha yavaş/);
});

test("a tier that does not fit is disabled with the reason; warnings are shown", () => {
  const eight = models({
    machine: { ...mac, total_ram_bytes: 8 * GB, available_ram_bytes: 4 * GB },
    options: [
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "recommended", { warnings: ["low_available_memory"] }),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "too_big_ram"),
    ],
    selected: "qwen3.5-4b-q4_k_m",
    warnings: ["single_option"],
  });
  const view = choiceView(eight);
  const [small, large] = view.options;
  assert.equal(large.eligible, false);
  assert.equal(large.badge, "Bellek yetersiz");
  assert.equal(large.reason, "Bu bilgisayarda 8 GB bellek var; bu model 9 GB istiyor.");
  assert.match(small.lowMemory, /Şu an boş bellek \(4 GB\) bu modelin ihtiyacından \(6,5 GB\) az/);
  assert.deepEqual(view.warnings, ["Bu bilgisayara yalnızca bir yanıt modeli uyuyor; daha büyük bir model için daha fazla bellek gerekir."]);
  assert.equal(choiceView(eight, "qwen3.5-9b-q4_k_m").selected, "qwen3.5-4b-q4_k_m", "a disabled tier is never picked");
  const disk = models({
    machine: { ...mac, free_disk_bytes: 4 * GB },
    options: [option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "too_big_disk")],
    selected: null,
    support: { status: "unsupported", code: "no_tier_fits", missing_cpu_features: [] },
  });
  assert.equal(choiceView(disk).options[0].reason, "Modeller klasöründe 4 GB boş yer var; bu model için 6 GB gerekiyor.");
});

test("an unsupported machine gets the message and nothing to pick", () => {
  const old = models({
    machine: { ...mac, metal: false, cpu_features: { avx2: false, fma: false, f16c: true, bmi2: true } },
    options: [option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "fits")],
    selected: null,
    support: { status: "unsupported", code: "cpu_unsupported", missing_cpu_features: ["avx2", "fma"] },
  });
  const view = choiceView(old);
  assert.equal(view.selected, null);
  assert.equal(view.showList, false);
  assert.equal(view.options[0].eligible, false);
  assert.match(view.unsupported.text, /\(AVX2, FMA\) desteklemiyor/);
  const small = models({
    machine: { ...mac, total_ram_bytes: 4 * GB },
    options: [
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "too_big_ram"),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "too_big_ram"),
    ],
    selected: null,
    support: { status: "unsupported", code: "no_tier_fits", missing_cpu_features: [] },
  });
  const none = choiceView(small);
  assert.equal(none.showList, true, "the list shows why nothing fits");
  assert.match(none.unsupported.text, /en küçük model 6,5 GB bellek istiyor/);
  setLanguage("en");
  assert.match(choiceView(small).unsupported.text, /the smallest needs 6.5 GB of memory/);
});

test("a server without options offers no choice", () => {
  assert.equal(choiceView({ download: { available: true, bytes: 1 } }), null);
  assert.equal(choiceView({ download: { options: [] } }), null);
  assert.equal(choiceView(null), null);
});
