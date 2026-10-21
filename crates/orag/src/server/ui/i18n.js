// The page's languages (step 77): every user-visible string, in Turkish and
// English, by stable dotted key. The page opens in Turkish unless English was
// chosen before (localStorage "orag.lang"). Text reaches the page through
// textContent and attributes only.

const STORAGE_KEY = "orag.lang";
/** The URL fragment pair the desktop app hands the language over in. */
const HASH_KEY = "lang";
/** When the choice was made (ms since 1970), in storage and the fragment. */
const AT_STORAGE_KEY = "orag.lang.at";
const AT_KEY = "at";
/**
 * Where the page reports a choice to the desktop app: a navigation the app
 * cancels and reads (apps/desktop/src/window.rs `language_report`).
 */
const REPORT_URL = "orag-lang://choice";
const LANGUAGES = ["tr", "en"];
const DEFAULT_LANGUAGE = "tr";
const LOCALES = { tr: "tr-TR", en: "en-US" };
const SVG_NS = "http://www.w3.org/2000/svg";
/** The attributes `applyStaticTexts` fills, each from `data-i18n-<name>`. */
const ATTRIBUTES = ["placeholder", "title", "aria-label"];

const tr = {
  // Language menu
  "language.label": "Dil",

  // Sidebar
  "sidebar.label": "Koleksiyonlar ve belgeler",
  "brand.tag": "Yerel belge asistanı",
  "collections.title": "Koleksiyonlar",
  "collections.new": "Yeni koleksiyon",
  "collections.nameLabel": "Yeni koleksiyon adı",
  "collections.namePlaceholder": "Koleksiyon adı",
  "collections.create": "Oluştur",
  "collections.nameHint": "En fazla 64 karakter: harf, rakam, boşluk, \"_\", \".\" veya \"-\".",
  "documents.title": "Belgeler",
  "upload.title": "Dosya yükle",
  "upload.hint": "Sürükleyip bırakın veya seçin · TXT, MD, DOCX, PDF",
  "upload.limit": " · en fazla {max} MB",
  "upload.clear": "Tamamlananları temizle",
  "documents.listLabel": "Seçili koleksiyondaki belgeler",
  "documents.empty": "Bu koleksiyonda henüz belge yok.",
  "documents.more": "Daha fazla göster",
  "help.open": "Nasıl kullanılır?",

  // Common
  "common.cancel": "Vazgeç",
  "common.confirm": "Onayla",
  "common.close": "Kapat",
  "common.stop": "Durdur",
  "common.deleted": "\"{name}\" silindi.",

  // Top bar
  "collection.reindex": "Yeniden indeksle",
  "collection.reindexTitle": "Belgeleri saklanan kopyalarından mevcut modelle yeniden işler",
  "collection.delete": "Sil",
  "collection.deleteTitle": "Koleksiyonu ve belgelerini siler",
  "offline.banner": "ORAG'a ulaşılamıyor. Sunucu kapalı ya da yeniden başlıyor olabilir; geri geldiğinde sayfa kendiliğinden devam eder.",

  // Setup panel
  "setup.title": "Modelleri kur",
  "setup.missing": "ORAG'ın çalışması için şu modeller gerekiyor:",
  "setup.downloadHint": "Modeller bir kez Hugging Face'ten indirilir; sonrasında ORAG internetsiz çalışır.",
  "setup.progressLabel": "İndirme ilerlemesi",
  "setup.total": "Toplam",
  "setup.offline.summary": "Model paketlerim var (çevrimdışı kurulum)",
  "setup.offline.hint1": "Model paketlerini internete bağlı bir bilgisayarda ",
  "setup.offline.hint2": " ile hazırlayın, klasörü bu bilgisayara kopyalayın ve yolunu aşağıya yazın. Paketler ",
  "setup.offline.hint3": " klasörüne kurulur.",
  "setup.pathLabel": "Model paketleri klasörü",
  "setup.import": "İçe aktar",
  "setup.pathWindows": "C:\\ORAG-paketleri",
  "setup.pathUnix": "~/orag-packs",
  "setup.askLocked": "Soru sormak için önce modelleri kurun.",
  "setup.restarting": "ORAG yeniden başlıyor…",
  "setup.restartTimeout": "ORAG bir dakika içinde geri gelmedi. Sunucu penceresindeki iletiyi kontrol edin.",
  "setup.manualRestart": "Modeller kuruldu; ORAG'ı kapatıp yeniden başlatın. Yeniden başladığında bu sayfa kendiliğinden yenilenir.",
  "setup.importing": "İçe aktarılıyor… Büyük model dosyaları birkaç dakika sürebilir.",
  "setup.imported": "İçe aktarıldı: {ids}.",
  "setup.alreadyInstalled": "Bu klasördeki paketler zaten kurulu.",
  "setup.stillMissing": "Hâlâ eksik: {ids}.",
  "setup.importFailed": "Model paketleri içe aktarılamadı.",
  "setup.importConflict": "Başka bir içe aktarma veya indirme sürüyor; bitmesini bekleyin.",
  "setup.importInvalid": "Bu klasörden model paketi içe aktarılamadı.",

  // The answer model choice (step 82)
  "setup.machine.title": "Bu bilgisayar",
  "setup.machine.memory": "Bellek",
  "setup.machine.memoryValue": "{total} (şu an boş: {available})",
  "setup.machine.cpu": "İşlemci",
  "setup.machine.cores": "Çekirdek",
  "setup.machine.coresValue": "{physical} fiziksel, {logical} mantıksal",
  "setup.machine.gpu": "Ekran kartı",
  "setup.machine.metal": "Apple GPU (Metal)",
  "setup.machine.disk": "Boş disk",
  "setup.machine.unknown": "bilinmiyor",
  "setup.tiers.legend": "Yanıt modeli",
  "setup.configuredNotDownloadable": "config.toml'daki {id} modeli ORAG içinden indirilemiyor; aşağıda seçtiğiniz model indirilip onun yerine ayarlanır.",
  "setup.tier.small": "Küçük",
  "setup.tier.default": "Standart",
  "setup.tier.large": "Büyük: daha yavaş, daha çok bellek kullanır",
  "setup.tier.current": "Şu anki: {id} (config.toml'da ayarlı)",
  "setup.tier.details": "{size} indirme · {ram}",
  "setup.tier.installedDetails": "Kurulu · {ram}",
  "setup.tier.recommendedRam": "Önerilen bellek: {ram}",
  "setup.tier.minimumRam": "En az {ram}",
  "setup.gib": "{value} GiB",
  "setup.tier.speedMetal": "Tipik yanıt süresi: Apple GPU'da yaklaşık {seconds} sn (tahmin)",
  "setup.tier.speedCpu": "4 çekirdekli bir GitHub test makinesinde ölçülen tipik süre: yaklaşık {seconds} sn",
  "setup.verdict.recommended": "Önerilen",
  "setup.verdict.fits": "Uygun",
  "setup.verdict.too_big_ram": "Bellek yetersiz",
  "setup.verdict.too_big_disk": "Disk yetersiz",
  "setup.reason.too_big_ram": "Bu bilgisayarda {total} bellek var; bu model en az {need} istiyor.",
  "setup.reason.too_big_disk": "Modeller klasöründe {free} boş yer var; bu model için {need} gerekiyor.",
  "setup.warning.single_option": "Bu bilgisayara yalnızca bir yanıt modeli uyuyor; daha büyük bir model için daha fazla bellek gerekir.",
  "setup.warning.slow_cpu": "Bu işlemcide bazı hızlandırıcı komutlar ({features}) yok; ORAG çalışır ama yanıtlar daha yavaş gelir.",
  "setup.warning.below_recommended_ram": "Bu bilgisayarın belleği önerilenin altında; yanıtlar yavaş olabilir.",
  "setup.warning.low_available_memory": "Şu an boş bellek ({available}) bu model için önerilen bellekten ({need}) az. İndirme yine de çalışır; soru sormadan önce başka uygulamaları kapatmanız gerekebilir.",
  "setup.unsupported.cpu_unsupported": "Bu bilgisayarın işlemcisi ORAG'ın gerektirdiği komutları ({features}) desteklemiyor; ORAG bu bilgisayarda çalışamaz.",
  "setup.unsupported.no_tier_fits": "Bu bilgisayarda ORAG'ın yanıt modelleri için yeterli bellek veya disk alanı yok (en küçük model en az {need} bellek istiyor). ORAG bu bilgisayarda çalışamaz.",

  // Model download
  "download.button": "Modelleri indir",
  "download.buttonSize": "Modelleri indir ({size})",
  "download.running": "Modeller indiriliyor… Bu sayfayı kapatsanız da indirme sürer.",
  "download.cancelled": "İndirme durduruldu. Kaldığı yerden devam edebilirsiniz.",
  "download.done": "Modeller indirildi ve doğrulandı.",
  "download.retry": "Tekrar dene",
  "download.resume": "Devam et",
  "download.installed": "Modeller zaten kurulu; indirilecek bir şey yok.",
  "download.unavailable": "Bu sunucu modelleri kendisi indiremiyor; model paketlerini aşağıdan çevrimdışı kurun.",
  "download.startFailed": "İndirme başlatılamadı.",
  "download.cancelFailed": "İndirme durdurulamadı.",
  "download.pollFailed": "İndirmenin durumu okunamadı.",
  "download.conflict": "Başka bir indirme veya içe aktarma sürüyor; bitmesini bekleyin.",
  "download.disabled": "Model indirme bu kurulumda kapatılmış (config.toml: models.download). Çevrimdışı kurulumu kullanın.",
  "download.notDownloadable": "Bu modeller ORAG içinden indirilemiyor. Çevrimdışı kurulumu kullanın.",
  "download.unknownModel": "Seçilen yanıt modeli bu sürümde sunulmuyor. Sayfayı yenileyip listeden seçin.",
  "download.notEligible": "Seçilen yanıt modeli bu bilgisayara artık uymuyor (bellek veya disk). Sayfayı yenileyip listeden uygun bir model seçin.",
  "download.state.waiting": "Sırada",
  "download.state.downloading": "İndiriliyor",
  "download.state.verifying": "Doğrulanıyor",
  "download.state.installed": "Kuruldu",
  "download.state.failed": "Başarısız",
  "download.error.network": "Hugging Face'e bağlanılamadı. İnternet bağlantınızı ve varsa proxy (vekil sunucu) ayarını kontrol edip tekrar deneyin.",
  "download.error.http": "Hugging Face isteği kabul etmedi. Biraz sonra tekrar deneyin.",
  "download.error.checksum": "İndirilen dosya bozuk çıktı ve silindi. Tekrar deneyin.",
  "download.error.disk": "Diskte yeterli boş alan yok. Yer açıp tekrar deneyin.",
  "download.error.cancelled": "İndirme durduruldu.",
  "download.error.internal": "Sunucuda bir hata oluştu; ayrıntılar sunucu günlüğünde.",
  "download.error.config": "Modeller kuruldu, ancak seçilen yanıt modeli config.toml dosyasına yazılamadı. Ayrıntılar aşağıda.",
  "download.error.fallback": "Modeller indirilemedi. Tekrar deneyin.",

  // Welcome and composer
  "welcome.title": "Belgelerinize soru sorun",
  "welcome.lead": "Yanıtlar yalnızca seçili koleksiyondaki belgelerden yazılır ve her bilgi kaynağına bağlanır.",
  "welcome.empty": "Bu koleksiyonda henüz belge yok. Başlamak için soldan bir dosya yükleyin.",
  "turns.label": "Sorular ve yanıtlar",
  "ask.label": "Sorunuz",
  "ask.placeholder": "Bir soru yazın…",
  "ask.send": "Gönder",
  "ask.hint.before": "",
  "ask.hint.after": " koleksiyonunda aranır · Enter gönderir, Shift + Enter yeni satır",
  "drop.title": "Dosyaları bırakın",
  "drop.target": "\"{name}\" koleksiyonuna eklenecek",

  // Answers
  "ask.answer": "Yanıt",
  "ask.searching": "Belgelerde aranıyor…",
  "ask.writing": "Yanıt yazılıyor…",
  "ask.copy": "Kopyala",
  "ask.copied": "Kopyalandı",
  "ask.copyFailed": "Kopyalanamadı",
  "ask.busy": "Başka bir yanıt üretiliyor; bir kez daha denenecek.",
  "ask.lost": "Bağlantı kesildi; yanıt tamamlanmadı.",
  "ask.stopped": "Yanıt durduruldu.",
  "ask.incomplete": "Yanıt tamamlanamadı.",
  "ask.failed": "Soru yanıtlanamadı.",
  "ask.retry": "Tekrar sor",
  "ask.pickCollection": "Önce soldan bir koleksiyon seçin.",
  "answer.abstained": "Belgelerde bu sorunun yanıtı bulunamadı.",
  "answer.uncited": "Bu yanıt hiçbir kaynağa atıf yapmıyor; belgelerden doğrulayın.",
  "answer.length": "Yanıt uzunluk sınırında kesildi.",
  "answer.repetition": "Yanıt kendini tekrar ettiği için durduruldu; eksik olabilir.",

  // Sources
  "sources.title": "Kaynaklar · {count}",
  "sources.more": "Devamını göster",
  "sources.less": "Kısalt",
  "sources.citation": "Kaynak {numbers}",
  "sources.missing": "Bu numarada bir kaynak yok",
  "sources.reviewed": "İncelenen {count} kaynak",
  "sources.reviewed.one": "İncelenen {count} kaynak",
  "sources.unused": "Yanıtta kullanılmayan {count} kaynak",
  "sources.unused.one": "Yanıtta kullanılmayan {count} kaynak",

  // Collections
  "collections.default": "Varsayılan",
  "collections.count": "{count} belge",
  "collections.count.one": "{count} belge",
  "collections.undeletable": "silinemez",
  "collections.loadFailed": "Koleksiyonlar yüklenemedi.",
  "collections.created": "\"{name}\" koleksiyonu oluşturuldu.",
  "collections.createFailed": "Koleksiyon oluşturulamadı.",
  "collections.exists": "Bu adla bir koleksiyon zaten var (büyük/küçük harf fark etmez).",
  "collections.invalidName": "Geçersiz ad: en fazla 64 harf, rakam, boşluk, \"_\", \".\" veya \"-\".",
  "collections.gone": "\"{name}\" artık yok.",
  "collections.deleteQuestion": "Koleksiyon silinsin mi?",
  "collections.deleteMessage": "\"{name}\" koleksiyonu ve içindeki {count} kalıcı olarak silinecek. Bu işlem geri alınamaz.",
  "collections.deleteConfirm": "Koleksiyonu sil",
  "collections.deleteFailed": "Koleksiyon silinemedi.",
  "collections.deleteDefault": "Varsayılan koleksiyon silinemez.",
  "collections.reindexQuestion": "Yeniden indekslensin mi?",
  "collections.reindexMessage": "\"{name}\" koleksiyonundaki belgeler, saklanan kopyalarından mevcut modelle yeniden işlenecek. Dosyaları tekrar yüklemeniz gerekmez; bitene kadar yanıtlar eksik olabilir.",
  "collections.reindexQueued": "{count} yeniden indeksleniyor.",
  "collections.reindexFailed": "Yeniden indeksleme başlatılamadı.",
  "collections.reindexBusy": "Bu koleksiyonda hâlâ işlenen belgeler var; bitince tekrar deneyin.",

  // Documents
  "documents.status.queued": "Sırada",
  "documents.status.indexing": "İşleniyor",
  "documents.status.ready": "Hazır",
  "documents.status.failed": "Hata",
  "documents.unnamed": "Belge #{id}",
  "documents.deleteLabel": "Sil: {name}",
  "documents.chunks": "{count} parça",
  "documents.chunks.one": "{count} parça",
  "documents.loadFailed": "Belge listesi yüklenemedi.",
  "documents.deleteQuestion": "Belge silinsin mi?",
  "documents.deleteMessage": "\"{name}\" belgesi \"{collection}\" koleksiyonundan kalıcı olarak silinecek.",
  "documents.deleteConfirm": "Belgeyi sil",
  "documents.deleteFailed": "Belge silinemedi.",

  // Uploads
  "upload.waiting": "Bekliyor",
  "upload.sending": "Gönderiliyor",
  "upload.removed": "Silindi",
  "upload.indexed": "{count} parça indekslendi.",
  "upload.indexed.one": "{count} parça indekslendi.",
  "upload.lostTrack": "Durum izlenemedi; belge listesine bakın.",
  "upload.noCollection": "Önce bir koleksiyon seçin.",
  "upload.tooLarge": "Dosya {size}; sınır {max} MB (config.toml: max_document_mb).",
  "upload.failed": "Dosya yüklenemedi.",
  "upload.duplicate": "Bu dosya bu koleksiyonda zaten vardı.",

  // Status bar
  "status.models": "Yanıt modeli · arama modeli",
  "status.reconnected": "Bağlantı geri geldi.",
  "gpu.line": "Ekran kartı: {device} · {state}",
  "gpu.unnamed": "adı bilinmiyor",
  "gpu.inUse": "kullanılıyor",
  "gpu.partial": "kısmen kullanılıyor ({on}/{total} katman)",
  "gpu.notUsedWhy": "Ekran kartı kullanılmıyor: {why}",
  "gpu.notUsedCode": "Ekran kartı kullanılmıyor ({code})",
  "gpu.reason.loader-missing": "Vulkan sürücüsü yok, işlemci kullanılıyor",
  "gpu.reason.no-eligible-device": "uygun kart yok, işlemci kullanılıyor",
  "gpu.reason.probe-timeout": "kart yanıt vermedi, işlemci kullanılıyor",
  "gpu.reason.start-timeout": "kartla açılamadı, işlemci kullanılıyor",
  "gpu.reason.start-failed": "kartla açılamadı, işlemci kullanılıyor",
  "gpu.reason.crashed-twice": "iki kez durdu, işlemci kullanılıyor",
  "gpu.reason.fit_failed": "modele yetecek boş bellek yok",
  "gpu.reason.fit_zero_layers": "modele yetecek boş bellek yok",
  "gpu.reason.alloc_failed": "kartın belleği doldu",
  "gpu.reason.no_eligible_device": "uygun kart yok",
  "gpu.retry": "Ekran kartını yeniden dene",
  "gpu.retryFailed": "Ekran kartı yeniden denenemedi.",
  "gpu.lastAnswer": "Son yanıt: {seconds} sn · {where}",
  "gpu.where.vulkan": "ekran kartı",
  "gpu.where.metal": "Apple GPU",
  "gpu.where.cpu": "işlemci",

  // API errors
  "error.unreachable": "Sunucuya ulaşılamıyor. orag serve çalışıyor mu?",
  "error.reindex_required": "Bu koleksiyon başka bir modelle indekslenmiş. Üst çubuktaki \"Yeniden indeksle\" ile belgeleri mevcut modelle yeniden işleyin.",
  "error.unsupported_format": "Bu dosya türü desteklenmiyor. TXT, Markdown, DOCX veya PDF yükleyin.",
  "error.too_large": "Dosya boyut sınırını aşıyor.",
  "error.upload_timeout": "Dosya zamanında gönderilemedi; tekrar deneyin.",
  "error.busy": "Sunucu şu an meşgul. Birkaç saniye sonra tekrar deneyin.",
  "error.shutting_down": "Sunucu kapanıyor.",
  "error.not_found": "Bulunamadı; başka bir yerden silinmiş olabilir.",
  "error.internal": "Sunucuda bir hata oluştu; ayrıntılar sunucu günlüğünde.",
  "error.models_missing": "Modeller henüz kurulu değil. Önce yukarıdaki \"Modelleri kur\" bölümünden modelleri indirin.",

  // Formatting
  "format.decimal": ",",
  "format.percent": "%{value}",
  "format.units": "bayt,KB,MB,GB,TB",
  "format.page": "s. {page}",
  "format.pages": "s. {pages}",
  "warning.ocr_required": "Taranmış sayfalar okunamadı",
  "warning.extraction_failed": "Bazı sayfalar okunamadı",
  "warning.suspicious_layout": "Bazı sayfaların metni bozuk olabilir",
  "warning.styles_unreadable": "Başlık stilleri okunamadı",
  "warning.no_text": "Metin bulunamadı",
  "warning.reason.unmapped": "eşleşmeyen karakterler",
  "warning.reason.split": "harflere bölünmüş kelimeler",

  // Help window
  "help.title": "ORAG'a hoş geldiniz",
  "help.lead": "Kendi belgelerinize soru sorabildiğiniz, tamamen bu bilgisayarda çalışan bir belge asistanı.",
  "help.intro": "ORAG yüklediğiniz belgeleri okur, sorduğunuz soruyu bu belgelerde arar ve yanıtı yalnızca bulduğu bilgilerle yazar. Her bilginin hangi belgeden geldiğini gösterir; belgelerde yanıt yoksa bunu açıkça söyler.",
  "help.privacy.title": "Her şey bu bilgisayarda kalır.",
  "help.privacy.text": "Belgeler, sorular ve yanıtlar internete gönderilmez; ORAG internet bağlantısı olmadan çalışır.",
  "help.steps": "Nasıl kullanılır?",
  "help.step1.title": "Koleksiyon seçin",
  "help.step1.text": "Belgeleri konulara göre ayırın (örneğin Sözleşmeler, İnsan Kaynakları). Soldaki \"+\" ile yeni koleksiyon oluşturun. Sorular yalnızca seçili koleksiyonda aranır.",
  "help.step2.title": "Dosya yükleyin",
  "help.step2.text": "TXT, Markdown, DOCX veya PDF dosyalarını soldaki \"Dosya yükle\" alanına bırakın ya da tıklayıp seçin. Durumu \"Hazır\" olan belgeler aranabilir.",
  "help.step3.title": "Soru sorun",
  "help.step3.text": "Sorunuzu alttaki kutuya yazıp Enter'a basın. Yanıttaki [1] gibi numaralara tıklayarak bilginin geldiği kaynağı görün.",
  "help.tips": "İpuçları",
  "help.tip1": "Taranmış (fotoğraf) PDF'lerin metni okunamaz; metin içeren PDF kullanın.",
  "help.tip2": "Yanıtlar belgelerden üretilir ama hata yapabilir; önemli kararlarda kaynağı kontrol edin.",
  "help.tip3": "Model değiştiyse koleksiyonu \"Yeniden indeksle\" ile güncelleyin; dosyaları tekrar yüklemeniz gerekmez.",
  "help.keys": "Kısayollar",
  "help.key.send": "Soruyu gönderir",
  "help.key.newline": "Yeni satır",
  "help.key.close": "Bu pencereyi kapatır",
  "help.later": "Bu pencereyi sol alttaki \"Nasıl kullanılır?\" ile yeniden açabilirsiniz.",
  "help.start": "Başlayalım",
};

const en = {
  "language.label": "Language",

  "sidebar.label": "Collections and documents",
  "brand.tag": "Local document assistant",
  "collections.title": "Collections",
  "collections.new": "New collection",
  "collections.nameLabel": "New collection name",
  "collections.namePlaceholder": "Collection name",
  "collections.create": "Create",
  "collections.nameHint": "Up to 64 characters: letters, digits, spaces, \"_\", \".\" or \"-\".",
  "documents.title": "Documents",
  "upload.title": "Upload files",
  "upload.hint": "Drag and drop or choose · TXT, MD, DOCX, PDF",
  "upload.limit": " · up to {max} MB",
  "upload.clear": "Clear finished",
  "documents.listLabel": "Documents in the selected collection",
  "documents.empty": "No documents in this collection yet.",
  "documents.more": "Show more",
  "help.open": "How to use",

  "common.cancel": "Cancel",
  "common.confirm": "Confirm",
  "common.close": "Close",
  "common.stop": "Stop",
  "common.deleted": "\"{name}\" deleted.",

  "collection.reindex": "Reindex",
  "collection.reindexTitle": "Processes the documents again from their stored copies with the current model",
  "collection.delete": "Delete",
  "collection.deleteTitle": "Deletes the collection and its documents",
  "offline.banner": "Cannot reach ORAG. The server may be stopped or restarting; the page carries on by itself when it is back.",

  "setup.title": "Install models",
  "setup.missing": "ORAG needs these models to work:",
  "setup.downloadHint": "The models are downloaded once from Hugging Face; after that ORAG works offline.",
  "setup.progressLabel": "Download progress",
  "setup.total": "Total",
  "setup.offline.summary": "Offline install: I have model packs",
  "setup.offline.hint1": "On a computer with internet access, prepare the model packs with ",
  "setup.offline.hint2": ", copy the folder to this computer and enter its path below. The packs are installed in ",
  "setup.offline.hint3": ".",
  "setup.pathLabel": "Model packs folder",
  "setup.import": "Import",
  "setup.pathWindows": "C:\\ORAG-packs",
  "setup.pathUnix": "~/orag-packs",
  "setup.askLocked": "Install the models first to ask questions.",
  "setup.restarting": "ORAG is restarting…",
  "setup.restartTimeout": "ORAG did not come back within a minute. Check the message in the server window.",
  "setup.manualRestart": "The models are installed; quit ORAG and start it again. This page reloads by itself once it is back.",
  "setup.importing": "Importing… Large model files can take a few minutes.",
  "setup.imported": "Imported: {ids}.",
  "setup.alreadyInstalled": "The packs in this folder are already installed.",
  "setup.stillMissing": "Still missing: {ids}.",
  "setup.importFailed": "Could not import the model packs.",
  "setup.importConflict": "Another import or download is running; wait for it to finish.",
  "setup.importInvalid": "No model pack could be imported from this folder.",

  // The answer model choice (step 82)
  "setup.machine.title": "This computer",
  "setup.machine.memory": "Memory",
  "setup.machine.memoryValue": "{total} ({available} free now)",
  "setup.machine.cpu": "Processor",
  "setup.machine.cores": "Cores",
  "setup.machine.coresValue": "{physical} physical, {logical} logical",
  "setup.machine.gpu": "Graphics",
  "setup.machine.metal": "Apple GPU (Metal)",
  "setup.machine.disk": "Free disk",
  "setup.machine.unknown": "unknown",
  "setup.tiers.legend": "Answer model",
  "setup.configuredNotDownloadable": "The model {id} set in config.toml cannot be downloaded from within ORAG; the model you pick below is downloaded and replaces it.",
  "setup.tier.small": "Small",
  "setup.tier.default": "Standard",
  "setup.tier.large": "Large: slower, uses more memory",
  "setup.tier.current": "Current: {id} (set in config.toml)",
  "setup.tier.details": "{size} download · {ram}",
  "setup.tier.installedDetails": "Installed · {ram}",
  "setup.tier.recommendedRam": "Recommended memory: {ram}",
  "setup.tier.minimumRam": "At least {ram}",
  "setup.gib": "{value} GiB",
  "setup.tier.speedMetal": "Typical answer time: about {seconds} s on the Apple GPU (estimate)",
  "setup.tier.speedCpu": "Measured median on a 4-vCPU GitHub runner: about {seconds} s",
  "setup.verdict.recommended": "Recommended",
  "setup.verdict.fits": "Fits",
  "setup.verdict.too_big_ram": "Not enough memory",
  "setup.verdict.too_big_disk": "Not enough disk",
  "setup.reason.too_big_ram": "This computer has {total} of memory; this model needs at least {need}.",
  "setup.reason.too_big_disk": "The models folder has {free} free; this model needs {need}.",
  "setup.warning.single_option": "Only one answer model fits this computer; a larger model needs more memory.",
  "setup.warning.slow_cpu": "This processor lacks some speed-up instructions ({features}); ORAG runs, but answers come more slowly.",
  "setup.warning.below_recommended_ram": "This computer has less memory than recommended; answers may be slow.",
  "setup.warning.low_available_memory": "Less memory is free right now ({available}) than recommended for this model ({need}). The download still works; you may need to close other apps before asking questions.",
  "setup.unsupported.cpu_unsupported": "This computer's processor lacks instructions ORAG needs ({features}); ORAG cannot run on this computer.",
  "setup.unsupported.no_tier_fits": "This computer does not have enough memory or disk space for ORAG's answer models (the smallest needs at least {need} of memory). ORAG cannot run on this computer.",

  "download.button": "Download models",
  "download.buttonSize": "Download models ({size})",
  "download.running": "Downloading the models… The download continues even if you close this page.",
  "download.cancelled": "Download stopped. You can resume where it left off.",
  "download.done": "The models are downloaded and verified.",
  "download.retry": "Try again",
  "download.resume": "Resume",
  "download.installed": "The models are already installed; there is nothing to download.",
  "download.unavailable": "This server cannot download the models itself; install the model packs offline below.",
  "download.startFailed": "Could not start the download.",
  "download.cancelFailed": "Could not stop the download.",
  "download.pollFailed": "Could not read the download status.",
  "download.conflict": "Another download or import is running; wait for it to finish.",
  "download.disabled": "Model download is turned off in this installation (config.toml: models.download). Use the offline install.",
  "download.notDownloadable": "These models cannot be downloaded from within ORAG. Use the offline install.",
  "download.unknownModel": "The chosen answer model is not offered by this version. Reload the page and pick one from the list.",
  "download.notEligible": "The chosen answer model no longer fits this computer (memory or disk). Reload the page and pick a model that fits.",
  "download.state.waiting": "Waiting",
  "download.state.downloading": "Downloading",
  "download.state.verifying": "Verifying",
  "download.state.installed": "Installed",
  "download.state.failed": "Failed",
  "download.error.network": "Could not connect to Hugging Face. Check your internet connection and proxy setting, if any, then try again.",
  "download.error.http": "Hugging Face did not accept the request. Try again in a little while.",
  "download.error.checksum": "The downloaded file was damaged and has been deleted. Try again.",
  "download.error.disk": "There is not enough free disk space. Free some space and try again.",
  "download.error.cancelled": "Download stopped.",
  "download.error.internal": "Something went wrong on the server; see the server log for details.",
  "download.error.config": "The models are installed, but the chosen answer model could not be written to config.toml. Details below.",
  "download.error.fallback": "Could not download the models. Try again.",

  "welcome.title": "Ask your documents",
  "welcome.lead": "Answers are written only from the documents in the selected collection, and every fact links to its source.",
  "welcome.empty": "No documents in this collection yet. Upload a file on the left to begin.",
  "turns.label": "Questions and answers",
  "ask.label": "Your question",
  "ask.placeholder": "Type a question…",
  "ask.send": "Send",
  "ask.hint.before": "Searching in ",
  "ask.hint.after": " · Enter sends, Shift + Enter adds a new line",
  "drop.title": "Drop files",
  "drop.target": "Will be added to \"{name}\"",

  "ask.answer": "Answer",
  "ask.searching": "Searching the documents…",
  "ask.writing": "Writing the answer…",
  "ask.copy": "Copy",
  "ask.copied": "Copied",
  "ask.copyFailed": "Could not copy",
  "ask.busy": "Another answer is being written; it will be tried once more.",
  "ask.lost": "The connection dropped; the answer is incomplete.",
  "ask.stopped": "Answer stopped.",
  "ask.incomplete": "The answer could not be completed.",
  "ask.failed": "The question could not be answered.",
  "ask.retry": "Ask again",
  "ask.pickCollection": "Choose a collection on the left first.",
  "answer.abstained": "The documents do not answer this question.",
  "answer.uncited": "This answer cites no source; check it against the documents.",
  "answer.length": "The answer was cut off at the length limit.",
  "answer.repetition": "The answer was stopped because it repeated itself; it may be incomplete.",

  "sources.title": "Sources · {count}",
  "sources.more": "Show more",
  "sources.less": "Show less",
  "sources.citation": "Source {numbers}",
  "sources.missing": "There is no source with this number",
  "sources.reviewed": "{count} sources reviewed",
  "sources.reviewed.one": "{count} source reviewed",
  "sources.unused": "{count} sources not used in the answer",
  "sources.unused.one": "{count} source not used in the answer",

  "collections.default": "Default",
  "collections.count": "{count} documents",
  "collections.count.one": "{count} document",
  "collections.undeletable": "cannot be deleted",
  "collections.loadFailed": "Could not load the collections.",
  "collections.created": "Collection \"{name}\" created.",
  "collections.createFailed": "Could not create the collection.",
  "collections.exists": "A collection with this name already exists (case does not matter).",
  "collections.invalidName": "Invalid name: up to 64 letters, digits, spaces, \"_\", \".\" or \"-\".",
  "collections.gone": "\"{name}\" no longer exists.",
  "collections.deleteQuestion": "Delete the collection?",
  "collections.deleteMessage": "The collection \"{name}\" and its {count} will be deleted for good. This cannot be undone.",
  "collections.deleteConfirm": "Delete collection",
  "collections.deleteFailed": "Could not delete the collection.",
  "collections.deleteDefault": "The default collection cannot be deleted.",
  "collections.reindexQuestion": "Reindex the collection?",
  "collections.reindexMessage": "The documents in \"{name}\" will be processed again from their stored copies with the current model. You do not need to upload the files again; answers may be incomplete until it finishes.",
  "collections.reindexQueued": "Reindexing {count}.",
  "collections.reindexFailed": "Could not start the reindex.",
  "collections.reindexBusy": "Documents in this collection are still being processed; try again when they are done.",

  "documents.status.queued": "Queued",
  "documents.status.indexing": "Processing",
  "documents.status.ready": "Ready",
  "documents.status.failed": "Failed",
  "documents.unnamed": "Document #{id}",
  "documents.deleteLabel": "Delete: {name}",
  "documents.chunks": "{count} chunks",
  "documents.chunks.one": "{count} chunk",
  "documents.loadFailed": "Could not load the document list.",
  "documents.deleteQuestion": "Delete the document?",
  "documents.deleteMessage": "\"{name}\" will be deleted from \"{collection}\" for good.",
  "documents.deleteConfirm": "Delete document",
  "documents.deleteFailed": "Could not delete the document.",

  "upload.waiting": "Waiting",
  "upload.sending": "Sending",
  "upload.removed": "Deleted",
  "upload.indexed": "{count} chunks indexed.",
  "upload.indexed.one": "{count} chunk indexed.",
  "upload.lostTrack": "Could not follow the status; check the document list.",
  "upload.noCollection": "Choose a collection first.",
  "upload.tooLarge": "The file is {size}; the limit is {max} MB (config.toml: max_document_mb).",
  "upload.failed": "Could not upload the file.",
  "upload.duplicate": "This file was already in this collection.",

  "status.models": "Answer model · search model",
  "status.reconnected": "Connection restored.",
  "gpu.line": "Graphics card: {device} · {state}",
  "gpu.unnamed": "unnamed",
  "gpu.inUse": "in use",
  "gpu.partial": "partly in use ({on}/{total} layers)",
  "gpu.notUsedWhy": "Graphics card not used: {why}",
  "gpu.notUsedCode": "Graphics card not used ({code})",
  "gpu.reason.loader-missing": "no Vulkan driver, using the processor",
  "gpu.reason.no-eligible-device": "no suitable card, using the processor",
  "gpu.reason.probe-timeout": "the card did not answer, using the processor",
  "gpu.reason.start-timeout": "could not start on the card, using the processor",
  "gpu.reason.start-failed": "could not start on the card, using the processor",
  "gpu.reason.crashed-twice": "it stopped twice, using the processor",
  "gpu.reason.fit_failed": "not enough free memory for the model",
  "gpu.reason.fit_zero_layers": "not enough free memory for the model",
  "gpu.reason.alloc_failed": "the card's memory filled up",
  "gpu.reason.no_eligible_device": "no suitable card",
  "gpu.retry": "Try the graphics card again",
  "gpu.retryFailed": "Could not retry the graphics card.",
  "gpu.lastAnswer": "Last answer: {seconds} s · {where}",
  "gpu.where.vulkan": "graphics card",
  "gpu.where.metal": "Apple GPU",
  "gpu.where.cpu": "processor",

  "error.unreachable": "Cannot reach the server. Is orag serve running?",
  "error.reindex_required": "This collection was indexed with another model. Use \"Reindex\" in the top bar to process its documents again with the current model.",
  "error.unsupported_format": "This file type is not supported. Upload TXT, Markdown, DOCX or PDF.",
  "error.too_large": "The file is over the size limit.",
  "error.upload_timeout": "The file was not sent in time; try again.",
  "error.busy": "The server is busy. Try again in a few seconds.",
  "error.shutting_down": "The server is shutting down.",
  "error.not_found": "Not found; it may have been deleted elsewhere.",
  "error.internal": "Something went wrong on the server; see the server log for details.",
  "error.models_missing": "The models are not installed yet. Download them first in \"Install models\" above.",

  "format.decimal": ".",
  "format.percent": "{value}%",
  "format.units": "bytes,KB,MB,GB,TB",
  "format.page": "p. {page}",
  "format.pages": "pp. {pages}",
  "warning.ocr_required": "Scanned pages could not be read",
  "warning.extraction_failed": "Some pages could not be read",
  "warning.suspicious_layout": "The text of some pages may be garbled",
  "warning.styles_unreadable": "Heading styles could not be read",
  "warning.no_text": "No text found",
  "warning.reason.unmapped": "unmapped characters",
  "warning.reason.split": "words split into single letters",

  "help.title": "Welcome to ORAG",
  "help.lead": "A document assistant that answers questions about your own documents and runs entirely on this computer.",
  "help.intro": "ORAG reads the documents you upload, looks up your question in them and writes the answer only from what it finds. It shows which document each fact comes from, and says so plainly when the documents have no answer.",
  "help.privacy.title": "Everything stays on this computer.",
  "help.privacy.text": "Documents, questions and answers are never sent to the internet; ORAG works without an internet connection.",
  "help.steps": "How to use",
  "help.step1.title": "Choose a collection",
  "help.step1.text": "Sort documents by topic (for example Contracts, Human Resources). Create a new collection with \"+\" on the left. Questions only search the selected collection.",
  "help.step2.title": "Upload files",
  "help.step2.text": "Drop TXT, Markdown, DOCX or PDF files on \"Upload files\" on the left, or click it to choose them. Documents marked \"Ready\" can be searched.",
  "help.step3.title": "Ask a question",
  "help.step3.text": "Type your question in the box at the bottom and press Enter. Click numbers such as [1] in the answer to see the source of a fact.",
  "help.tips": "Tips",
  "help.tip1": "The text of scanned (photo) PDFs cannot be read; use PDFs that contain text.",
  "help.tip2": "Answers come from the documents but can be wrong; check the source for important decisions.",
  "help.tip3": "If the model changed, update the collection with \"Reindex\"; you do not need to upload the files again.",
  "help.keys": "Shortcuts",
  "help.key.send": "Sends the question",
  "help.key.newline": "New line",
  "help.key.close": "Closes this window",
  "help.later": "Open this window again with \"How to use\" at the bottom left.",
  "help.start": "Let's start",
};

export const MESSAGES = { tr, en };

const isLanguage = (lang) => LANGUAGES.includes(lang);

/** A time in ms since 1970 as written in storage or a fragment, else null. */
function timeOf(text) {
  return /^\d{1,15}$/.test(String(text ?? "")) ? Number(text) : null;
}

/**
 * The choice a `#lang=tr|en[&at=<ms>]` fragment hands over: its language
 * and when it was made (null for the app's default, no choice seen yet).
 */
export function choiceFromHash(hash) {
  const pairs = new Map(String(hash ?? "").replace(/^#/, "").split("&").map((pair) => pair.split("=")));
  const language = pairs.get(HASH_KEY);
  return isLanguage(language) ? { language, at: timeOf(pairs.get(AT_KEY)) } : null;
}

/** The language a `#lang=` fragment names, or null. */
export const languageFromHash = (hash) => choiceFromHash(hash)?.language ?? null;

/**
 * Which of two choices stands: a timed choice beats an untimed one, the
 * later time wins (a tie goes to the hand-over), and between two untimed
 * ones the stored choice stays. So a stale hand-over from the desktop app
 * never downgrades a newer choice made on the page, and the app's default
 * never overrides a stored one. Turkish when there is neither.
 */
export function chooseLanguage(handed, stored) {
  if (!handed || !stored) return handed ?? stored ?? { language: DEFAULT_LANGUAGE, at: null };
  if (handed.at === null) return stored;
  if (stored.at === null) return handed;
  return handed.at >= stored.at ? handed : stored;
}

const currentHash = () => globalThis.location?.hash ?? "";

function storedChoice() {
  try {
    const language = globalThis.localStorage?.getItem(STORAGE_KEY);
    return isLanguage(language) ? { language, at: timeOf(globalThis.localStorage.getItem(AT_STORAGE_KEY)) } : null;
  } catch {
    return null; // Storage blocked.
  }
}

/**
 * The choice to open in: the one handed over in the URL fragment (the
 * desktop app does that, its page origin may change between launches) or
 * the stored one, by `chooseLanguage`; else Turkish.
 */
const initialChoice = () => chooseLanguage(choiceFromHash(currentHash()), storedChoice());

export const initialLanguage = () => initialChoice().language;

/** The choice the fragment hands over (the desktop app's, or a reload's), or null. */
const handedChoice = choiceFromHash(currentHash());
/**
 * Opened with a `#lang=` fragment: the page then keeps its own choice in it,
 * so a reload opens in that choice even when storage is blocked. A page
 * opened without one never gets a fragment.
 */
const followsFragment = handedChoice !== null;
/**
 * Inside the desktop app: its window marks every page before the page's own
 * scripts run (apps/desktop/src/window.rs DESKTOP_MARKER_SCRIPT). Only then
 * is a choice reported to the app (`reportChoice`); a fragment alone never
 * enables it, so a plain browser never navigates to the app's report scheme.
 */
const inDesktopApp = globalThis.__ORAG_DESKTOP__ === true;
let chosen = initialChoice();
let current = chosen.language;

export const getLanguage = () => current;

/** The locale for numbers and dates in the current language. */
export const locale = () => LOCALES[current];

function lookup(lang, key, params) {
  const table = MESSAGES[lang];
  if (params?.count === 1 && Object.hasOwn(table, `${key}.one`)) return table[`${key}.one`];
  return Object.hasOwn(table, key) ? table[key] : null;
}

/** Whether `key` is a message (the fallback language has every key). */
export const hasMessage = (key) => Object.hasOwn(MESSAGES[DEFAULT_LANGUAGE], key);

/** A `{ key, params }` message, as `toast` and placeholder values take it. */
export const isMessage = (value) => typeof value === "object" && value !== null && typeof value.key === "string";

function fill(value) {
  return isMessage(value) ? t(value.key, value.params) : String(value);
}

/**
 * The string for `key` in the current language with `{name}` placeholders
 * filled from `params`; falls back to Turkish, then to the key itself. A
 * placeholder value may itself be a `{ key, params }` message, translated
 * in the same language (so stored params never pin an older language).
 */
export function t(key, params) {
  const text = lookup(current, key, params) ?? lookup(DEFAULT_LANGUAGE, key, params) ?? key;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name) => (Object.hasOwn(params, name) ? fill(params[name]) : whole));
}

function argsOf(node) {
  try {
    return JSON.parse(node.dataset.i18nArgs || "null") || undefined;
  } catch {
    return undefined;
  }
}

/** Writes `text` into `node` and keeps its icon (icons.js puts it first). */
function writeText(node, text) {
  const icons = [...node.children].filter((child) => child.namespaceURI === SVG_NS);
  node.replaceChildren(...icons, text);
}

function attributeName(dataKey) {
  return `data-i18n-${dataKey}`;
}

/**
 * Fills every element under `root` (and `root` itself) that names a key:
 * `data-i18n` (text), `data-i18n-placeholder`, `data-i18n-title` and
 * `data-i18n-aria-label`; `data-i18n-args` holds the placeholders as JSON.
 */
export function applyStaticTexts(root = globalThis.document) {
  if (!root) return;
  const selector = ["[data-i18n]", ...ATTRIBUTES.map((a) => `[${attributeName(a)}]`)].join(",");
  const nodes = [...(root.matches?.(selector) ? [root] : []), ...root.querySelectorAll(selector)];
  for (const node of nodes) {
    const args = argsOf(node);
    if (node.hasAttribute("data-i18n")) writeText(node, t(node.getAttribute("data-i18n"), args));
    for (const attr of ATTRIBUTES) {
      const key = node.getAttribute(attributeName(attr));
      if (key) node.setAttribute(attr, t(key, args));
    }
  }
}

function setArgs(node, params) {
  if (params) node.dataset.i18nArgs = JSON.stringify(params);
  else delete node.dataset.i18nArgs;
}

/**
 * Shows `key` in `node` and keeps it translated on a language change; a null
 * key empties the node. `params` must not depend on the language.
 */
export function setText(node, key, params) {
  if (!key) {
    node.removeAttribute("data-i18n");
    setArgs(node, null);
    node.textContent = "";
    return;
  }
  node.setAttribute("data-i18n", key);
  setArgs(node, params);
  writeText(node, t(key, params));
}

/** Like `setText` for one of the translated attributes (e.g. "title"). */
export function setAttrText(node, attr, key, params) {
  node.setAttribute(attributeName(attr), key);
  setArgs(node, params);
  node.setAttribute(attr, t(key, params));
}

function store({ language, at }) {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, language);
    if (at === null) globalThis.localStorage?.removeItem(AT_STORAGE_KEY);
    else globalThis.localStorage?.setItem(AT_STORAGE_KEY, String(at));
  } catch {
    // Not remembered across reloads; the page still switches now.
  }
}

/**
 * Writes `#lang=<language>[&at=<ms>]` into the page's own address without a
 * navigation: page state for a reload only, the app never reads it.
 */
function writeFragment({ language, at }) {
  const loc = globalThis.location;
  const fragment = `${HASH_KEY}=${language}${at === null ? "" : `&${AT_KEY}=${at}`}`;
  try {
    globalThis.history?.replaceState(globalThis.history.state ?? null, "", `${loc.pathname}${loc.search}#${fragment}`);
  } catch {
    // A reload then opens in the stored choice.
  }
}

/**
 * Tells the desktop app about a timed choice the moment it stands, by a
 * navigation to `orag-lang://choice?lang=<tr|en>&at=<ms>` that the app
 * cancels: the page stays as it is (no unload; requests keep running). Only
 * in the app (`inDesktopApp`); an untimed choice is never reported.
 */
function reportChoice({ language, at }) {
  if (!inDesktopApp || at === null || !globalThis.location) return;
  try {
    globalThis.location.href = `${REPORT_URL}?${HASH_KEY}=${language}&${AT_KEY}=${at}`;
  } catch {
    // The app keeps the choice it knew.
  }
}

/**
 * The time for a new choice: now, but always after the standing choice, so a
 * clock set back after a choice cannot make the newer choice lose.
 */
function nextTime(standing) {
  const now = Date.now();
  return standing && standing.at !== null && standing.at >= now ? standing.at + 1 : now;
}

/** Shows the page in `lang` without storing it. */
function apply(lang) {
  current = lang;
  const doc = globalThis.document;
  if (doc) {
    doc.documentElement.lang = lang;
    applyStaticTexts(doc);
  }
  globalThis.window?.dispatchEvent(new CustomEvent("orag:language", { detail: lang }));
}

/**
 * Switches to `lang` ("tr" or "en"), remembers it, re-translates the page
 * and announces it with `orag:language` on window. Returns the language in
 * use (unchanged for an unknown one).
 */
export function setLanguage(lang) {
  if (!isLanguage(lang)) return current;
  chosen = { language: lang, at: nextTime(chosen) };
  store(chosen);
  if (followsFragment) writeFragment(chosen);
  reportChoice(chosen);
  apply(lang);
  return current;
}

/**
 * Translates the page into the starting language and wires the menu. Opened
 * with a fragment, the standing choice is stored and written back into it,
 * and reported to the app when it is not the one handed over: the page's
 * own choice was newer.
 */
export function initLanguage(select) {
  if (followsFragment) {
    store(chosen);
    writeFragment(chosen);
    if (chosen.language !== handedChoice.language || chosen.at !== handedChoice.at) reportChoice(chosen);
  }
  apply(current);
  if (!select) return;
  select.value = current;
  select.addEventListener("change", () => {
    select.value = setLanguage(select.value);
  });
}
