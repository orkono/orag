# v0.4 holdout set audit

The v0.4 holdout set (`v04-holdout-retrieval.jsonl`, 100 questions; `v04-holdout-answers.jsonl`, 30
probes) over `eval/corpus/v02` was written by `claude-opus-5-5` with no access to the v0.2 rows except
a list of their families and evidence passages, which it had to avoid (a validator rejects any reused
family and any label overlapping a v0.2 label). `gpt-6-sol` (high effort), a model of another family
that did not write it, then checked all 130 rows against the documents: 3 wrong, 20 weak,
0 nit. Every replacement row was applied. Not human-checked.

| id | severity | problem |
|---|---|---|
| v4-fac-004 | weak | en-pump-manual.docx also says to store the pump indoors when the site can fall below 2 °C; the label ends before that action. |
| v4-fac-011 | weak | tr-izin-yonetmeligi.md applies this rule only to a single sickness report exceeding ten days and also requires a return-to-work fitness approval; “uzun” hides the threshold. |
| v4-fac-015 | weak | en-warranty-terms.md starts the 15-business-day clock on receipt of complete documentation, or after a required on-site inspection; the current label omits the clock condition. |
| v4-fac-018 | weak | tr-sulama-katalogu.pdf has a stricter unopened-and-undamaged-package return condition; the generic “iade edeceğim ürün” does not distinguish it from tr-kargo-politikasi.md. |
| v4-xl-003 | weak | en-warranty-terms.md permits an on-site technician only for claims whose invoiced value exceeds 5,000 EUR; the question and label omit that condition. |
| v4-md-003 | weak | tr-izin-yonetmeligi.md says the three-business-day request window is for one- or two-day annual leave; other short leave types have different rules. |
| v4-md-006 | weak | tr-sulama-katalogu.pdf says to remove the timers’ batteries and store the devices where they will not freeze; the label includes only battery removal. |
| v4-md-012 | weak | “Normal return” is ambiguous: tr-kargo-politikasi.md requires original packaging and no use, while tr-sulama-katalogu.pdf requires unopened, undamaged packaging. Name the policy whose condition is sought. |
| v4-id-007 | weak | tr-anayasa.txt Article 28 contains more than press freedom and censorship; “md. 28 ne diyor?” asks for the whole article but the label gives only its opening clause. |
| v4-tab-004 | weak | en-pump-manual.docx describes a self-test initiated by pressing RESET and MODE; “otomatik testi” suggests it runs automatically. |
| v4-tab-006 | weak | tr-fiyat-listesi.txt lists both DS-3140 and DS-3150 hydrocyclones, but the catalogue table provides body material only for DS-3140; “hidrosiklonun” is ambiguous. |
| v4-acc-003 | wrong | tr-kargo-politikasi.md requires original packaging and an unused product; it does not say whether an opened original box is acceptable. tr-sulama-katalogu.pdf says unopened only for its own returns. |
| v4-acc-010 | weak | tr-anayasa.txt excepts military judges from the 65-year rule and leaves their age limit to statute; “hakimler” includes them. |
| v4-inf-008 | weak | tr-insan-kaynaklari.md requires a training cost above 30,000 TL and voluntary departure within twelve months of completion; “pahalı” and “sonra” leave out both conditions. |
| v4-una-004 | weak | tr-orx7-kurulum.md defines ORX-E221 as a database connection failure but gives no remedy. A generic “hangi adımlar” invites plausible troubleshooting from outside the corpus to be scored as an answer. |
| v4-ans-004 | weak | en-warranty-terms.md starts the decision period after a required on-site inspection; complete documents alone do not always start the 15-business-day clock. |
| v4-ans-010 | weak | A response giving only the catalogue’s three years passes the single expect even if it gets the export document wrong. Ask for the two units separately. |
| v4-ans-013 | weak | en-pump-manual.docx states 120 minutes; the current expect misses the equally correct “two hours” and “2 hours” answers. |
| v4-ans-015 | wrong | tr-izin-yonetmeligi.md awards 22 iş günü; expecting “22 gün” also accepts an incorrect 22-calendar-day answer. |
| v4-ans-018 | weak | tr-anayasa.txt sets 65 for judges and prosecutors but excepts military judges; the broad query overlooks that carve-out. |
| v4-ans-019 | wrong | “yargı yolu” also appears in the wrong answer “yargı yolu kapalı,” and a correct “evet, başvurabilir” answer need not contain it. |
| v4-ans-020 | weak | tr-insan-kaynaklari.md requires voluntary resignation within twelve months after completing training costing over 30,000 TL; the probe omits those conditions. |
| v4-ans-024 | weak | The retrieval family asks for the DS-3120 carton weight, while this probe asks for the filter’s own weight; tr-sulama-katalogu.pdf gives neither. |

## Review fixes (step 60 code review, gpt-6-sol)

- `v4-md-002` / `v4-ans-012` asked for the DS-5020 warranty, a fact the v0.2 set measures
  (`v2-tab-024`, `v2-ans-030`); they now ask for its price and the wall-mount bracket in the box
  (family `md-ds5020-fiyat-kutu`).
- `v4-tab-005` and `v4-acc-001` now say "belirsiz süreli, müdür olmayan": the notice table applies
  to open-ended contracts, and managers get ten weeks regardless of tenure.

The served-default baselines stayed hybrid in context 0.909 and expected facts 0.950.
