ORAG - Windows hızlı başlangıç / Windows quick start
=====================================================

TÜRKÇE

ORAG belgelerinizi bu bilgisayarda indeksler ve onlardan kaynak göstererek
cevap verir. İnternete kendiliğinden bağlanmaz; modelleri yalnızca siz
"Modelleri indir"e bastığınızda indirir, ya da yerel paketlerden kurulur.

1. Başlat menüsünden "ORAG"ı açın. Bir konsol penceresi açılır (ORAG'ın
   kayıtları burada görünür) ve tarayıcıda http://127.0.0.1:2442 açılır.
   Pencereyi kapatmak ORAG'ı durdurur; yarım kalan indeksleme bir sonraki
   açılışta devam eder.
2. İlk açılışta sayfa modelleri kurmanızı ister.
   - İnternet varsa: "Modelleri indir" düğmesine basın. İki model (yaklaşık
     3,1 GB) indirilir ve doğrulanır; bekleyin. ORAG kendiliğinden yeniden
     başlar. Bağlantı koparsa düğmeye yeniden basın; kaldığı yerden devam
     eder.
   - İnternet yoksa: model paketlerini başka bir bilgisayarda hazırlayıp
     (USB bellek gibi) bu bilgisayara kopyalayın. Sayfadaki "Modelleri kur"
     alanına o klasörü yazın ve "İçe aktar"a basın. ORAG kendiliğinden
     yeniden başlar.
   Başlat menüsündeki "ORAG modellerini indir (internet)" kısayolu da aynı
   paketleri %USERPROFILE%\ORAG-paketleri klasörüne indirir (önce onay
   ister).
3. Sol taraftan belge yükleyin (TXT, Markdown, DOCX, PDF; en fazla 10 MB) ve
   alttaki kutudan soru sorun.

Belgeleriniz ve modelleriniz %USERPROFILE%\.orag klasöründedir. ORAG'ı
kaldırmak bu klasöre dokunmaz.

Windows SmartScreen uyarısı: ORAG henüz kod imzalı değildir. Kurulumda
"Windows bilgisayarınızı korudu" çıkarsa "Ek bilgi" ve ardından "Yine de
çalıştır"a basın.

ORAG yalnızca bu bilgisayardan (127.0.0.1) erişilebilir ve parola istemez;
aynı bilgisayardaki başka hesaplar da kullanabilir. Kişisel bilgisayarlar
içindir.

ENGLISH

ORAG indexes your documents on this computer and answers questions from them
with cited sources. It never connects to the internet on its own: it
downloads the models only when you press "Modelleri indir", or installs them
from local packs.

1. Open "ORAG" from the Start menu. A console window opens (ORAG's log) and
   the browser opens http://127.0.0.1:2442. Closing the window stops ORAG;
   indexing that was cut off resumes on the next start.
2. On the first start the page asks for the models. With internet, press
   "Modelleri indir" (download models): the two models (about 3.1 GB) are
   downloaded and verified; wait, and ORAG restarts by itself. If the
   connection breaks, press it again; it continues where it stopped.
   Without internet, copy packs built elsewhere (for example on a USB
   drive), type that folder into "Modelleri kur" (install models) and press
   "İçe aktar"; ORAG restarts by itself. The Start menu shortcut "ORAG
   modellerini indir (internet)" also downloads the packs into
   %USERPROFILE%\ORAG-paketleri after asking.
3. Upload documents in the sidebar (TXT, Markdown, DOCX, PDF; up to 10 MB)
   and ask in the box at the bottom.

Documents and models live in %USERPROFILE%\.orag; uninstalling keeps them.

SmartScreen: ORAG is not code-signed yet. If Windows says it protected your
PC, choose "More info", then "Run anyway".

ORAG is reachable only from this computer (127.0.0.1) and asks for no
password; other accounts on the same computer can use it too. It is meant for
personal computers.

-----------------------------------------------------------------------------
Release checklist on a real Windows 10/11 PC (no developer tools), for the
owner before publishing an installer:
  [ ] install; SmartScreen "More info / Run anyway"
  [ ] Start menu "ORAG" opens the console and the page in setup mode
  [ ] "Modelleri indir" downloads both models with progress; ORAG restarts
      by itself and the page is ready
  [ ] "Modelleri kur" imports packs from a USB folder; ORAG restarts
  [ ] upload the DS2026 PDF parts; ask a question; sources show pages
  [ ] close the console; start again; the documents are still there
  [ ] uninstall; %USERPROFILE%\.orag is kept
