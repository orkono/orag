// ORAG built-in page (D-021): entry point. Plain DOM modules, no libraries;
// the sections talk through `orag:*` events on `document` (collections.js).
import { initAsk } from "./ask.js";
import { initBench } from "./bench.js";
import { initCollections } from "./collections.js";
import { initDocuments } from "./documents.js";
import { initHelp } from "./help.js";
import { initLanguage } from "./i18n.js";
import { applyIcons } from "./icons.js";
import { initSetup } from "./setup.js";
import { initStatus } from "./status.js";
import { initUpload } from "./upload.js";

applyIcons();
// The page text in the chosen language before any section draws its own.
initLanguage(document.getElementById("language-select"));
// The other sections listen before the collection list first loads.
initAsk();
initUpload();
initDocuments();
initCollections();
initStatus();
initHelp();
initBench();
initSetup();
