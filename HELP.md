# RasterFit — uputstvo za OpenCADStudio

## 1. Namena

RasterFit povezuje raster crtež i native OpenCADStudio geometriju. Plugin detektuje centerline-ove na slici, kalibriše piksele u stvarne jedinice i fituje selektovanu geometriju uz raster. Sve obrade se izvršavaju lokalno.

Podržan je **OpenCADStudio 2026.44, API v14**. Gotov paket u ovom repozitorijumu je za **Linux x86_64**.

## 2. Instalacija i pokretanje

Iz korena repozitorijuma:

```bash
mkdir -p ~/.config/OpenCADStudio/plugins
cp -r release/com.rasterfit ~/.config/OpenCADStudio/plugins/
```

Restartuj OCS. Ako je paket pravilno učitan, postoji ribbon kartica **RasterFit**.

Dock panel se namerno ne otvara pri startu. Otvori ga komandom **Status** ili `RF_STATUS` kada želiš da vidiš kalibraciju, segmente, masku i poslednji fit.

## 3. Preporučen tok rada

1. Otvori raster ili PDF.
2. Kalibriši najmanje jedan element poznate dužine.
3. Po potrebi niveliši sliku, postavi origin i uključi guides.
4. Podesi kontrast ako centerline nije dovoljno jasan.
5. Nacrtaj ili selektuj native geometriju.
6. Po potrebi ograniči fit maskom.
7. Pokreni Preview fit za kontrolu, zatim Fit za primenu.
8. Proveri heatmap i sačuvaj DXF.

## 4. Raster, PDF i podešavanje slike

| Alat / komanda | Funkcija |
|---|---|
| **Open raster**, `RF_OPEN` | Učitava PNG, JPEG ili WebP kao native OCS raster. Sa ribbon-a otvara file picker; iz komandne linije prihvata putanju. |
| **Open PDF**, `RF_OPEN_PDF` | Rasterizuje PDF stranicu za native rad. |
| **Previous page**, `RF_PAGE_PREV` | Prethodna PDF stranica. |
| **Next page**, `RF_PAGE_NEXT` | Sledeća PDF stranica. |
| **Image settings**, `RF_IMAGE_ADJUST` | Otvara podešavanja Brightness, Contrast, Sharpness, Invert i Opacity. |

PDF stranice se početno renderuju na 150 DPI. Pri promeni stranice ponovo proveri kalibraciju i masku.

Image settings koristi originalne piksele kao osnovu, pa se naknadne promene ne akumuliraju. Brightness/Contrast/Sharpness/Invert menjaju gray8 ulaz za ridge detekciju i fit. Opacity menja prikaz rastera, ne njegovu geometriju.

RasterFit šalje raster na `DRAWORDER BACK` pri otvaranju i restore-u, da ne prekriva linije. Mask i guide overlay-i se postavljaju na `FRONT`.

## 5. Kalibracija

### Calibrate

**Calibrate** / `RF_CALIBRATE` pokreće dvotačkovni picker sa ridge snap-om. Izaberi dve tačke na raster elementu čija je stvarna dužina poznata. Zatim unesi:

- **Real length** — stvarna dužina u mm;
- **Orientation** — Free, Horizontal, Vertical ili Angle;
- **World angle** — kada je izabrana Angle orijentacija.

Možeš dodati više kalibracionih segmenata. Plugin prikazuje skalu, rotaciju, maksimalni rezidual, Leave-One-Out dijagnostiku kada je dostupna i upozorenja kao što su premalo segmenata ili prekratak segment.

Brza varijanta:

```text
RF_CALIBRATE 100
```

Ovo odmah postavlja poznatu dužinu od 100 mm i preskače poseban dijalog za dužinu.

### Level

**Level** / `RF_LEVEL` poravnava izabranu raster liniju sa najbližom horizontalnom ili vertikalnom osom. Klikni dve tačke duž linije; koristi se isti ridge snap. Skala i kalibracioni segmenti ostaju sačuvani. Poravnanje radi i sa anizotropnom skalom.

### Set origin

**Set origin** / `RF_ORIGIN` postavlja novu world poziciju donjeg levog ugla rastera. Ovo je translacija RasterFit raster origin-a, ne promena globalnog OCS UCS-a.

- `sx`, `sy` i rotacija se ne menjaju;
- maske i guides prate origin;
- nova vrednost se čuva u DXF workspace-u.

`RF_ORIGIN_RESET` vraća origin na `[0, 0]`.

### Guides

**Guides** / `RF_GUIDES` uključuje ili isključuje dve native `XLINE` ose kroz origin:

- crvena horizontalna X osa;
- zelena vertikalna Y osa.

Guides su reference-only, ne ulaze u običnu selekciju i fit, i čuvaju se u dokumentu.

## 6. Snap na raster i crtanje

| Alat | Ponašanje |
|---|---|
| **Snap line**, `RF_LINE` | Dve tačke prave native `LINE`. |
| **Snap polyline**, `RF_POLYLINE` | Više tačaka prave native `LWPOLYLINE`; Enter završava otvorenu poliliniju. |

Oba alata koriste sub-pixel ridge snap u poluprečniku **8 raster piksela**, snap marker i live preview. Ako ridge nije u tom poluprečniku, koristi se kliknuta tačka.

OCS plugin API trenutno nema globalni custom snap provider. Zato ovi alati ne menjaju ponašanje standardnih OCS `LINE` i `PLINE` komandi.

## 7. Maske

Maska ograničava koji raster pikseli učestvuju u fitu. Include stroke može promeniti osnovu maske u “ništa nije uključeno”, dok Exclude uklanja oblasti. Feather između oblasti je 3 px.

| Alat | Upotreba |
|---|---|
| **Include**, `RF_MASK_INCLUDE` | Klikni tačke include četkice; Enter završava potez. |
| **Exclude**, `RF_MASK_EXCLUDE` | Klikni tačke exclude četkice; Enter završava potez. |
| **Rectangle**, `RF_MASK_RECT` | Izaberi dva naspramna ugla. |
| **Polygon**, `RF_MASK_POLYGON` | Unesi najmanje 3 temena; Enter zatvara polygon. |
| **Erase mask**, `RF_MASK_ERASE` | Dodaje exclude brush potez; ne briše originalne vektorske stroke definicije. |
| **From selection**, `RF_MASK_FROM_SELECTION` | Pretvara selektovane native `LINE` entitete u include brush segmente. |
| **Clear mask**, `RF_MASK_CLEAR` | Briše sve stroke-ove i njihov prikaz. |
| **Brush size**, `RF_MASK_BRUSH` | Postavlja radius Include/Exclude/Eraser četkice na 1–80 raster px. Podrazumevano je 6 px. |

Tokom editovanja vidi se kursor i kontura aktivnog poteza. Posle završetka, stroke-ovi ostaju prikazani kao poluprovidljivi native hatch: zeleno za Include, crveno za Exclude/Eraser.

Hatch je vizuelni prikaz pojedinačnih stroke-ova; preklapajuće oblasti mogu izgledati tamnije. Fit ne koristi hatch, nego originalne vektorske mask stroke-ove i izvedenu per-pixel mapu.

Checkbox **Mask enabled** u dock panelu uključuje/isključuje uticaj maske na fit. Vidljivost postojećeg overlay prikaza nije isto što i aktivacija maske za fit.

## 8. Native constraints

**Coincident** / `RF_CONSTRAINT_COINCIDENT` i **Tangent** / `RF_CONSTRAINT_TANGENT` kreiraju native OCS constraint iz selekcije.

Fitter čita podržane native `Assoc2dConstraintGroup` relacije: coincident, horizontal/vertical, parallel, perpendicular, equal, distance/X/Y, radius/diameter, angle, fixed point, point-on-curve i tangent, uključujući podržane polyline endpoint/segment reference.

Nepodržane ili nevažeće relacije ne zaustavljaju ceo dokument: prijavljuju se kao **skipped**. Native fit radi samo sa relacijama koje može tačno da mapira.

## 9. Fit i fit opcije

Pre fita selektuj native `LINE`, `LWPOLYLINE`, `CIRCLE` ili `ARC` geometriju u XY ravni. Polyline može imati prave i bulge/luk segmente. Reference-only mask i guide entiteti se ne fituju.

Podrazumevani fit je **rigidan**: menja poziciju/orijentaciju, ali čuva oblik, dužine, poluprečnike, handle, layer i vizuelna svojstva.

**Fit options** / `RF_FIT_OPTIONS` otvara:

| Opcija | Podrazumevano | Značenje |
|---|---:|---|
| Fit geometry (constrained) | isključeno | Dozvoljava constrained promenu geometrije. |
| Fit rotation | uključeno | Fit globalne rotacije. |
| Fit translation | uključeno | Fit globalne translacije. |
| Fit background scale | isključeno | Dozvoljava fit pozadinske skale. |
| Include construction geometry | isključeno | Uključuje construction geometriju. |
| Capture radius | 12 px | Maksimalna udaljenost za sparivanje sa ridge tačkama. |
| Robust scale | 1.5 | Jačina robustnog tretmana outlier-a; 0 isključuje. |
| Tangent weight | 0.1 | Težina tangentnog usklađivanja. |

Rezultat prikazuje RMS u pikselima, coverage, broj iteracija i konvergenciju. Atomic update znači da se selektovani entiteti menjaju zajedno; undo/redo koristi native OCS istoriju.

### Preview fit

**Preview fit** / `RF_FIT_PREVIEW` prikazuje rezultat kao privremene preview linije bez upisa u dokument.

**Enter samo zatvara preview. Ne primenjuje fit.** Posle toga pokreni **Fit** / `RF_FIT` za stvarnu izmenu.

## 10. Heatmap, izveštaji i OCS alati

- **Heatmap** / `RF_HEATMAP` — boji fitovanu geometriju prema RMS-u i coverage upozorenjima. Zeleno označava mali RMS, žuto srednji, crveno veći RMS ili nizak coverage. Ponovno pokretanje vraća originalne boje.
- **Export report** / `RF_EXPORT_REPORT` — upisuje poslednji fit kao JSON ili CSV kroz native file picker.
- **Distance** / `RF_MEASURE_DISTANCE` — pokreće native OCS `DIST`.
- **Angle** / `RF_MEASURE_ANGLE` — pokreće native OCS merenje ugla.
- **Hatch** / `RF_HATCH` — pokreće native OCS `HATCH`.
- **Status** / `RF_STATUS` — otvara/osvežava RasterFit dock panel i ispisuje stanje.

## 11. Čuvanje i ponovno otvaranje

RasterFit čuva workspace u DXF Named Object Dictionary XRecord-u. Obuhvaćeno je:

- izvor i PDF stranica;
- raster handle, origin, skala i rotacija;
- kalibracioni segmenti;
- mask stroke-ovi, enabled status i brush radius;
- image i fit opcije;
- guide i mask overlay reference;
- poslednji fit i povezani native handle-i.

Originalna slika ili PDF nisu ugrađeni u DXF. **Ne premeštaj i ne briši source fajl** ako želiš da workspace bude rekonstruisan posle reopen-a. Native raster i overlay entiteti ostaju u dokumentu, a reference-only oznake plugin ponovo primenjuje pri učitavanju.

## 12. Najčešći problemi

### RasterFit kartica ne postoji

- Proveri da koristiš OCS 2026.44/API v14.
- Proveri da postoje `~/.config/OpenCADStudio/plugins/com.rasterfit/plugin.toml` i `librf_ocs_plugin.so`.
- Potpuno restartuj OCS posle kopiranja ili zamene biblioteke.

### Fit prijavljuje da geometrija nije selektovana

Selektuj podržani native entitet, ne raster, mask hatch ili guide. Za polyline koristi 2D OCS `LWPOLYLINE`.

### Fit ne pronalazi centerline

Povećaj kontrast ili sharpness, proveri da raster nije prazan i da maska ne isključuje ciljanu oblast. Po potrebi isključi **Mask enabled** za dijagnostiku.

### Snap ne radi sa standardnim OCS LINE/PLINE

To je ograničenje host API-ja. Koristi **Snap line** ili **Snap polyline** iz RasterFit Draw grupe.

### Maska se ne vidi

Koristi aktuelni paket, završi stroke sa Enter ili drugim rectangle klikom i proveri da je `RASTERFIT-MASK` layer uključen. Fit koristi vektorsku masku čak i kada je njen vizuelni prikaz neželjen; Clear mask uklanja i definiciju i prikaz.

### Linije su ispod rastera

Aktuelni paket raster postavlja na `DRAWORDER BACK` pri otvaranju/reopen-u. Proveri i da `RASTERFIT-RASTER` layer nije zaključan ili sakriven.

### Workspace se ne vraća posle reopen-a

Proveri da originalni raster/PDF postoji na istoj putanji i da je dokument sačuvan posle RasterFit operacije. Stari dokumenti bez RasterFit XRecord-a nemaju workspace za restore.

## 13. Tehnička ograničenja

- Samo XY native fit; nema 3D polyline/general curve fita.
- Circle/Arc zahtevaju izotropnu kalibraciju.
- Nepodržane constraint relacije se prijavljuju kao skipped.
- Raster snap nije globalni OCS object snap.
- Gotova biblioteka je vezana za Linux x86_64 i OCS API v14 wire schema.
- Native plugin nije dostupan u browser verziji OCS-a.

## 14. Izvorni kod

Izvori i SVG ikonice su u `crates/rf-ocs-plugin/`. Ovaj plugin-only repozitorijum ne sadrži `rf-core` workspace potreban za rebuild. Gotov paket je u `release/com.rasterfit/`.

Adapter je označen kao **GPL-3.0-only**.
