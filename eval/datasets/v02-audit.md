# v0.2 evaluation set: audit sample

Status: synthetic, automatically validated, audited row by row by a model of another family; no human audit.

How the set was made (step 49, 2026-10-13; it replaces the Task 42 set):

- The corpus (`eval/corpus/v02`) is unchanged from Task 42.
- The 200 questions and 60 answer probes were written from scratch by `claude-opus-5-5`, given
  the corpus text and the rules (counts per category, dev/holdout balance, one split per family
  and per evidence string, evidence that occurs once in its document, unanswerable questions
  checked against general rules). It did not see the earlier set.
- `gpt-6-sol` (through `codex-orkono-hr`), which did not write the rows, audited all 260 rows
  against the documents: does the evidence fully answer the question, is better evidence
  elsewhere, is an unanswerable question answered by any rule, does the row fit its category,
  does a probe's `expect` reject wrong answers. It reported 43 rows (1 wrong, 42 weak), each
  with a full replacement row; all 43 replacements were applied. The wrong row (`v2-acc-020`)
  asked for a detention limit that the constitution gives three ways (48 hours, four days for
  collective offences, travel time excluded).
- A script then checked the counts, the split balance, the family and evidence split rule and
  that each evidence string occurs once in its document; `orag eval retrieval` and
  `orag eval answers` checked every label against the production-parsed corpus. One label that
  crossed a PDF line break was shortened.

40 questions below, 5 per category, fixed seed 42. `audited` marks rows the audit changed.

| id | category | split | question | evidence | audited |
|---|---|---|---|---|---|
| v2-acc-001 | accentless | dev | hijyen urunu iade edebilirmiyim | tr-kargo-politikasi.md: Hijyen ürünlerinde iade kabul edilmez |  |
| v2-acc-004 | accentless | holdout | pazartesi uzaktan calisma gunu secebilir miyim | tr-insan-kaynaklari.md: pazartesi uzaktan çalışma günü olarak seçilemez |  |
| v2-acc-008 | accentless | holdout | yurtdisi gorevde gunluk harcirah kac euro | tr-insan-kaynaklari.md: Yurt dışı görevlerde günlük harcırah 90 EUR olarak belirlenmiştir |  |
| v2-acc-009 | accentless | dev | sehir ici is seyahatinde taksi hangi durumlarda karsilaniyor | tr-insan-kaynaklari.md: Şehir içi iş seyahatlerinde taksi kullanımı yalnızca saat 22:00'den sonra veya ağır ekipman taşınması halinde karşılanır; diğer durumlarda toplu taşıma bedeli ödenir. | yes |
| v2-acc-017 | accentless | dev | kireclenen hatta yuzde 33 hidroklorik asit suya ne oranda katilir | tr-sulama-katalogu.pdf: Kireçlenme görülen hatlarda yüzde otuz üç derişimli hidroklorik asit, sulama suyuna binde bir oranında enjekte edilerek en az 30 dakika sistemde tutulur. | yes |
| v2-xl-004 | cross_language | holdout | Pompanın kolisi hasarlı geldi, satıcıya kaç saat içinde haber vermem gerekiyor? | en-pump-manual.docx: must be reported to the dealer within 48 hours of delivery |  |
| v2-xl-005 | cross_language | dev | Pompa su basabilecek bir çukura kurulabilir mi, koruma sınıfı ne? | en-pump-manual.docx: The pump is rated IP44 and may be splashed but not submerged |  |
| v2-xl-018 | cross_language | holdout | How many unused vacation days can I carry over into the next leave year? | tr-izin-yonetmeligi.md: en fazla beş iş günü bir sonraki izin yılına devreder |  |
| v2-xl-022 | cross_language | holdout | How much RAM and how many CPU cores does the ORX-7 server need? | tr-orx7-kurulum.md: ORX-7 en az 8 GB bellek ve 4 çekirdekli bir işlemci gerektirir |  |
| v2-xl-024 | cross_language | holdout | Which offences are listed in the SUÇ field of Court of Cassation ruling 2016/347? | tr-yargitay-1cd-2016-347.pdf: SUÇ : Kasten öldürmeye teşebbüs, iştirak halinde kasten yaralama | yes |
| v2-fac-002 | factual | holdout | Torbalı'daki fabrikanın kapalı alanı kaç metrekare? | tr-sulama-katalogu.pdf: 14.000 m² kapalı alanda faaliyet göstermektedir |  |
| v2-fac-003 | factual | dev | Deniz Sulama'nın teknik destek hattı hangi saatlerde açık? | tr-sulama-katalogu.pdf: hafta içi 08:30-18:00 saatleri arasında hizmet verir |  |
| v2-fac-006 | factual | holdout | Hidrosiklonun altındaki kum haznesi kaç litre alıyor? | tr-sulama-katalogu.pdf: Alt kısımdaki toplama haznesi 6 litre kapasitelidir |  |
| v2-fac-028 | factual | holdout | TBMM'de toplam kaç milletvekili bulunuyor? | tr-anayasa.txt: genel oyla seçilen altıyüz milletvekilinden oluşur |  |
| v2-fac-038 | factual | holdout | Deadline for reporting a security incident? | en-security-policy.md: reported to the security team within 4 hours of discovery |  |
| v2-id-003 | identifier | dev | AquaBoost E 25 error | en-pump-manual.docx: E-25 \| Motor winding temperature above 85 °C (latched) |  |
| v2-id-007 | identifier | dev | Barista 3000 shows E 9 | en-barista-manual.md: Error code E9 indicates the grinder is blocked |  |
| v2-id-008 | identifier | holdout | orx e104 hatası | tr-orx7-kurulum.md: ORX-E104 hatası, lisans anahtarının süresinin dolduğunu gösterir |  |
| v2-id-017 | identifier | dev | does DSS-WT-EN-2026/1 supersede the 2025 terms | en-warranty-terms.md: These terms replace document DSS-WT-EN-2025/2 |  |
| v2-id-020 | identifier | holdout | 2015/3839 esas sayılı dosyanın karar numarası | tr-yargitay-1cd-2016-347.pdf: 1. Ceza Dairesi 2015/3839 E. , 2016/347 K. |  |
| v2-inf-001 | inflection | dev | Mirasa ilişkin hakkım anayasal güvence altında mı? | tr-anayasa.txt: Herkes, mülkiyet ve miras haklarına sahiptir. |  |
| v2-inf-007 | inflection | dev | Temiz bir çevrede yaşamak anayasal bir hak mı? | tr-anayasa.txt: Herkes, sağlıklı ve dengeli bir çevrede yaşama hakkına sahiptir. |  |
| v2-inf-008 | inflection | holdout | Vergilendirmede mali güç dikkate alınıyor mu? | tr-anayasa.txt: kamu giderlerini karşılamak üzere, malî gücüne göre, vergi ödemekle yükümlüdür |  |
| v2-inf-014 | inflection | dev | Doğum yapacak çalışana doğumdan önce ve sonra kaçar hafta izin veriliyor? | tr-izin-yonetmeligi.md: Doğum izni, doğumdan önce sekiz ve doğumdan sonra sekiz hafta |  |
| v2-inf-018 | inflection | dev | Ay içinde kaç gecikmeden sonra uyarılırım? | tr-insan-kaynaklari.md: Bir ay içinde üç kez ve daha fazla geç kalan çalışana ekip lideri tarafından yazılı hatırlatma yapılır. | yes |
| v2-md-001 | multi_document | dev | DS-1600 borunun garanti süresi katalogda ve ihracat garanti şartlarında aynı mı? | tr-sulama-katalogu.pdf: DS-1600 damla borusu için 2 yıl garanti verilmektedir / en-warranty-terms.md: DS-1600 \| Round drip pipe 16 mm, 2 l/h \| 24 months |  |
| v2-md-006 | multi_document | dev | Does the DS-3140 hydrocyclone get the same warranty in the Turkish catalogue and the export terms? | tr-sulama-katalogu.pdf: DS-3140 hidrosiklon gövdesi için garanti süresi 5 yıldır / en-warranty-terms.md: DS-3140 \| Hydrocyclone sand separator 2" \| 60 months |  |
| v2-md-009 | multi_document | holdout | DS-4210 solenoid valve: price incl. VAT and export warranty length? | tr-sulama-katalogu.pdf: DS-4210 solenoid vananın fiyatı 1.350,00 TL KDV hariç, 1.620,00 TL KDV dahildir / en-warranty-terms.md: DS-4210 \| Solenoid valve 1", 24 VAC \| 24 months |  |
| v2-md-015 | multi_document | dev | Mağazada ücretsiz kargo kaç TL'den başlıyor, Deniz Sulama'da ücretsiz nakliye limiti ne? | tr-kargo-politikasi.md: 500 TL ve üzeri \| Ücretsiz / tr-sulama-katalogu.pdf: KDV hariç 25.000 TL ve üzerindeki siparişlerde Türkiye içi nakliye ücretsizdir |  |
| v2-md-019 | multi_document | holdout | Lost company laptop at Kuzey Lojistik: reporting deadline, versus the security policy's incident reporting deadline? | tr-insan-kaynaklari.md: en geç yirmi dört saat içinde Bilgi Teknolojileri yardım masasına bildirilir / en-security-policy.md: reported to the security team within 4 hours of discovery |  |
| v2-tab-005 | table | dev | how often to clean the inlet strainer | en-pump-manual.docx: Monthly \| Remove and rinse the inlet strainer |  |
| v2-tab-009 | table | dev | 20 yıllık çalışan kaç gün yıllık izin kullanır? | tr-izin-yonetmeligi.md: 15 yıl ve daha fazla \| 28 iş günü |  |
| v2-tab-011 | table | dev | Ev değiştirirken izin hakkım var mı? | tr-izin-yonetmeligi.md: Taşınma \| 1 iş günü \| Yeni adres beyanı |  |
| v2-tab-014 | table | holdout | Elek filtrenin dayanabileceği en yüksek basınç kaç bar? | tr-sulama-katalogu.pdf: DS-3120 \| Elek filtre \| 1 1/2" \| 8 \| 15 \| Polipropilen |  |
| v2-tab-023 | table | dev | Export warranty on the 8 mil flat drip tape? | en-warranty-terms.md: DS-1640 \| Flat drip tape 16 mm, 8 mil \| 12 months |  |
| v2-una-003 | unanswerable | dev | Barista Pro 3000'de E8 hatası ne anlama geliyor? | (none: unanswerable) |  |
| v2-una-004 | unanswerable | holdout | ORX-7 kurulumu için ne kadar disk alanı gerekiyor? | (none: unanswerable) |  |
| v2-una-007 | unanswerable | dev | Taşınma izni yılda kaç kez kullanılabilir? | (none: unanswerable) |  |
| v2-una-011 | unanswerable | dev | Siparişler hangi kargo firmasıyla gönderiliyor? | (none: unanswerable) |  |
| v2-una-025 | unanswerable | holdout | What is Brightwater's technical support phone number? | (none: unanswerable) |  |
