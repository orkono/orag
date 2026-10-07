// ORAG built-in page (D-021): entry point. Plain DOM modules, no libraries;
// the sections talk through `orag:*` events on `document` (collections.js).
import { initAsk } from "./ask.js";
import { initCollections } from "./collections.js";
import { initDocuments } from "./documents.js";
import { initUpload } from "./upload.js";

// The other sections listen before the collection list first loads.
initAsk();
initUpload();
initDocuments();
initCollections();
