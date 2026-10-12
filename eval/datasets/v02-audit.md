# v0.2 evaluation set: audit sample

Status: synthetic, automatically validated, reviewed by a second model; no human audit.

How the set was made (Task 42, 2026-10-12):

- Corpus documents and the fact list (394 facts with exact evidence) were written by `claude-fable-5-1`
  (`gpt-6-sol`, the first choice, was out of quota). Every evidence string was checked against the
  production-parsed corpus with `orag eval retrieval`.
- The 200 questions and 60 answer probes were written by `claude-fable-5-1` in a separate run, then
  reviewed row by row by another `claude-fable-5-1` run that had not written them. The review
  changed 32 rows (20 questions, 12 probes), among them six "unanswerable" questions that
  `en-warranty-terms.md` does answer ("products not named in section 2 carry no warranty").
- The unanswerable questions after the cut-off of that review (`v2-una-015`…`025`,
  `v2-ans-051`…`060`) were checked by grepping the corpus for their entity and attribute.

40 questions below, 5 per category, fixed seed 42. `reviewed` marks rows the review changed.

| id | category | split | question | evidence | reviewed |
|---|---|---|---|---|---|
| v2-acc-004 | accentless | dev | ds-4200 kuresel vana contasi hangi malzeme | tr-sulama-katalogu.pdf: Küresel vananın conta malzemesi PTFE'dir |  |
| v2-acc-001 | accentless | holdout | ds-1600 rulosu kac metre | tr-sulama-katalogu.pdf: 400 metrelik rulolar halinde sevk edilir |  |
| v2-acc-009 | accentless | dev | haftalik calisma suresi kac saat | tr-insan-kaynaklari.md: Haftalık normal çalışma süresi kırk beş saattir |  |
| v2-acc-008 | accentless | dev | ihracat siparisleri hangi limandan yukleniyor | tr-sulama-katalogu.pdf: İzmir Alsancak Limanı'ndan FOB olarak yüklenir |  |
| v2-acc-017 | accentless | dev | izin bakiyemi nereden gorebilirim | tr-izin-yonetmeligi.md: portalın "İzinlerim" sekmesinden görüntüler |  |
| v2-xl-005 | cross_language | dev | What is the overtime pay rate at Kuzey Lojistik? | tr-insan-kaynaklari.md: yüzde elli artırılmış tutarı üzerinden ödenir |  |
| v2-xl-024 | cross_language | holdout | Kahve makinesi E7 hatası veriyor, ne demek? | en-barista-manual.md: E7 means the water tank is not seated correctly |  |
| v2-xl-004 | cross_language | dev | How long is the probation period for new hires? | tr-insan-kaynaklari.md: deneme süresi iki aydır |  |
| v2-xl-022 | cross_language | dev | Garanti kapsamında değiştirilen ürün için garanti süresi yeniden mi başlıyor? | en-warranty-terms.md: carry only the remaining warranty period of the original product |  |
| v2-xl-018 | cross_language | holdout | Pompanın koruma sınıfı nedir, su sıçramasına dayanır mı? | en-pump-manual.docx: Protection class / IP44 / en-pump-manual.docx: rated IP44 and may be splashed but not submerged | yes |
| v2-fac-006 | factual | holdout | Hidrosiklonun altındaki kum haznesi kaç litre? | tr-sulama-katalogu.pdf: toplama haznesi 6 litre kapasitelidir |  |
| v2-fac-038 | factual | dev | Siyasi parti kurmak için devletten izin almak gerekiyor mu? | tr-anayasa.txt: Siyasî partiler önceden izin almadan kurulurlar |  |
| v2-fac-028 | factual | holdout | YSK kaç asıl üyeden oluşur? | tr-anayasa.txt: Yüksek Seçim Kurulu yedi asıl ve dört yedek üyeden oluşur |  |
| v2-fac-003 | factual | holdout | Teknik destek hattını hangi saatlerde arayabilirim? | tr-sulama-katalogu.pdf: hafta içi 08:30-18:00 saatleri arasında |  |
| v2-fac-002 | factual | holdout | Torbalı'daki üretim tesisinin kapalı alanı ne kadar? | tr-sulama-katalogu.pdf: 14.000 m² kapalı alanda |  |
| v2-id-003 | identifier | dev | E 23 keeps coming up, below what voltage does it trigger? | en-pump-manual.docx: Supply undervoltage below 198 V |  |
| v2-id-007 | identifier | dev | pompa sürekli E-27 veriyor ne yapmalıyım | en-pump-manual.docx: Leak suspected: repeated pressure loss while idle |  |
| v2-id-008 | identifier | holdout | E28 flow sensor | en-pump-manual.docx: Flow sensor paddle stuck or disconnected |  |
| v2-id-017 | identifier | holdout | DS 1640 rulo kaç metre | tr-sulama-katalogu.pdf: Rulolar 1.500 metre olarak sarılır |  |
| v2-id-020 | identifier | holdout | ds 3140 kaç kilo | tr-sulama-katalogu.pdf: ürün ağırlığı 38 kg'dır |  |
| v2-inf-001 | inflection | holdout | Seyahat özgürlüğü kimlere tanınmıştır? | tr-anayasa.txt: Herkes, yerleşme ve seyahat hürriyetine sahiptir |  |
| v2-inf-018 | inflection | dev | Zorla çalıştırmak yasak mıdır? | tr-anayasa.txt: Hiç kimse zorla çalıştırılamaz |  |
| v2-inf-007 | inflection | holdout | Suç sayılmayan bir fiil için ceza verilebilir mi? | tr-anayasa.txt: kanunun suç saymadığı bir fiilden dolayı |  |
| v2-inf-014 | inflection | dev | Çalışmak herkesin hakkı mıdır? | tr-anayasa.txt: Çalışma, herkesin hakkı ve ödevidir |  |
| v2-inf-008 | inflection | dev | Kişi düşünce ve kanaatini nasıl açıklayabilir? | tr-anayasa.txt: Herkes, düşünce ve kanaatlerini söz, yazı, resim veya başka yollarla |  |
| v2-md-015 | multi_document | holdout | For a DS-1600 warranty claim, what is the nominal emitter flow and what share of emitters may fall outside tolerance? | tr-sulama-katalogu.pdf: DS-1600 / 16 / 1,00 / 2,0 / 33 / en-warranty-terms.md: more than 5 percent of the emitters in a 100 metre sample |  |
| v2-md-019 | multi_document | holdout | Does the export warranty cover UV damage on DS-1640, and how is its UV stabilisation marked on the tape? | en-warranty-terms.md: not marked as UV-stabilised in the catalogue / tr-sulama-katalogu.pdf: ürün üzerine mavi çizgiyle işaretlenir |  |
| v2-md-009 | multi_document | dev | DS-3100 filtre standart kaç mesh diskle geliyor ve 200 mesh yedek set kaç TL? | tr-sulama-katalogu.pdf: 120 mesh elek inceliği ile standart olarak teslim edilir / tr-fiyat-listesi.txt: Yedek disk seti 2", 200 mesh |  |
| v2-md-001 | multi_document | dev | Ücretsiz izin almak istiyorum: kaç gün önceden başvurmalıyım ve kim onaylıyor? | tr-izin-yonetmeligi.md: en az otuz gün önce yazılı olarak yapılır / tr-insan-kaynaklari.md: yalnızca Genel Müdür tarafından onaylanabilir |  |
| v2-md-006 | multi_document | dev | How many days of paternity leave are there and does it need a manager's approval? | tr-izin-yonetmeligi.md: Eşin doğum yapması (babalık izni) / 5 iş günü / tr-insan-kaynaklari.md: babalık ve benzeri) için onay aranmaz |  |
| v2-tab-023 | table | dev | Evlat edinme durumunda kaç gün izin veriliyor? | tr-izin-yonetmeligi.md: Evlat edinme / 3 iş günü |  |
| v2-tab-014 | table | holdout | how much does the aquaboost weigh | en-pump-manual.docx: Net weight / 9.8 kg |  |
| v2-tab-011 | table | dev | DS-5010 ünitesinde pil ne kadar dayanıyor? | tr-sulama-katalogu.pdf: DS-5010 / 4 / 1 x 9 V alkalin pil / 12 ay | yes |
| v2-tab-009 | table | dev | 80 mesh diskle filtre en fazla ne kadar debi geçirir? | tr-sulama-katalogu.pdf: 80 / 200 / 30 |  |
| v2-tab-005 | table | dev | Bir torba DS-2050 birleştiricide kaç adet var? | tr-sulama-katalogu.pdf: DS-2050 / Geçmeli birleştirici / 16 / 6 / Polipropilen / 100 |  |
| v2-una-007 | unanswerable | dev | DS-1620 şeridi UV stabilizatörlü mü? | (none: unanswerable) |  |
| v2-una-025 | unanswerable | dev | Çocuğumun düğünü için kaç gün izin alabilirim? | (none: unanswerable) |  |
| v2-una-011 | unanswerable | holdout | DS-3120 elek filtrenin koli ağırlığı ne kadar? | (none: unanswerable) |  |
| v2-una-004 | unanswerable | dev | ds-5010 vana kablosu en fazla kaç metre olabilir | (none: unanswerable) | yes |
| v2-una-003 | unanswerable | dev | DS-3120 elek filtrenin yüksekliği kaç cm? | (none: unanswerable) | yes |
