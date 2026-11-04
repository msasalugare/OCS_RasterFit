# RasterFit za OpenCADStudio

RasterFit je native OpenCADStudio plugin za kalibraciju skeniranih crteža, crtanje sa sub-pixel snap-om na raster i fitovanje postojeće CAD geometrije na detektovane centerline-ove. Radi direktno u OCS dokumentu, bez web browsera, serverskog dela i telemetrije.

- **Podržana verzija:** OpenCADStudio **2026.44**, plugin API **v14**
- **Gotov paket:** **Linux x86_64**
- **Verzija plugina:** 0.1.0

Detaljan vodič: **[HELP.md](HELP.md)**.

## Instalacija gotovog plugina

Paket se nalazi u [`release/com.rasterfit/`](release/com.rasterfit/) i sadrži:

- `librf_ocs_plugin.so` — native Linux biblioteku;
- `plugin.toml` — OCS manifest i registraciju komandi.

Iz korena repozitorijuma:

```bash
mkdir -p ~/.config/OpenCADStudio/plugins
cp -r release/com.rasterfit ~/.config/OpenCADStudio/plugins/
```

Potpuno zatvori i ponovo pokreni OpenCADStudio. Posle učitavanja trebalo bi da postoji ribbon kartica **RasterFit**. Dock panel se ne otvara automatski; otvori ga preko **RasterFit → Analyze → Status**.

### Nadogradnja i uklanjanje

Za nadogradnju zameni postojeći `com.rasterfit` direktorijum novim paketom i restartuj OCS. Za uklanjanje:

```bash
rm -rf ~/.config/OpenCADStudio/plugins/com.rasterfit
```

## Brzi početak

1. **Open raster** — otvori PNG, JPEG ili WebP, ili **Open PDF** za PDF.
2. **Calibrate** — izaberi dve tačke na elementu poznate stvarne dužine i unesi dužinu. Za brzi unos može `RF_CALIBRATE 100`, gde je `100` dužina u mm.
3. Po potrebi koristi **Level** za poravnanje raster linije sa najbližom horizontalnom ili vertikalnom osom.
4. **Set origin** — postavi donji levi origin rastera; **Guides** uključuje X/Y ose.
5. Nacrtaj geometriju standardnim OCS alatima ili koristi **Snap line** / **Snap polyline** za ridge snap na raster.
6. Selektuj native geometriju i pokreni **Fit**.
7. Proveri **Heatmap**, po potrebi izvezi JSON/CSV izveštaj i sačuvaj DXF.

**Važno:** Enter u fit preview režimu samo zatvara preview. Za upis rezultata zatim pokreni posebno **Fit**.

## Šta plugin obuhvata

### Raster i PDF

- PNG/JPEG/WebP import kao native OCS `RasterImage`;
- višestranični PDF sa Previous/Next navigacijom;
- kalibracija skale i rotacije, ridge snap i dijagnostika reziduala;
- Level, podesivi raster origin i native X/Y guides;
- Brightness, Contrast, Sharpness, Invert i Opacity;
- raster draw order iza vektorske geometrije.

### Crtanje i fit

- Snap line i Snap polyline sa sub-pixel ridge snap-om;
- native `LINE`, `LWPOLYLINE`, `CIRCLE` i `ARC`;
- podrška za polyline bulge/lukove;
- rigidni fit koji podrazumevano čuva oblik, dužine i poluprečnike;
- opcioni constrained geometry fit;
- čuvanje handle-a, layera, vizuelnih svojstava i native constraint veza;
- atomic update i standardni OCS undo/redo.

### Maske

- Include i Exclude brush, Rectangle, Polygon i Eraser;
- podesivi brush radius od 1 do 80 raster piksela;
- maska iz selektovanih native linija;
- trajni poluprovidljivi hatch prikaz i posle editovanja;
- maska se koristi u fitu kao per-pixel include/exclude weight mapa.

### Analiza i dokument

- fit preview bez upisa geometrije;
- RMS/coverage rezultati i heatmap sa vraćanjem originalnih boja;
- native Coincident i Tangent komande, uz čitanje podržanih OCS constraint grupa;
- Distance, Angle i Hatch preko native OCS alata;
- JSON/CSV izveštaji;
- dock panel sa stanjem rastera, kalibracije, segmenata i poslednjeg fita;
- čuvanje workspace-a u DXF NOD XRecord: origin, kalibracija, maske, brush radius, guides, image settings, fit opcije i poslednji rezultat.

## Osnovna ograničenja

- Gotova biblioteka je samo za Linux x86_64 i OCS 2026.44/API v14. Drugi OCS release može zahtevati novi build.
- Fit podržava XY geometriju; krugovi i lukovi zahtevaju izotropnu kalibraciju.
- OCS nema globalni plugin snap provider, pa ridge snap radi kroz RasterFit Snap alate, a ne kroz obične OCS `LINE`/`PLINE` komande.
- Nepodržane native constraint relacije se preskaču i prijavljuju.
- Originalna slika/PDF mora ostati na istoj putanji da bi workspace mogao da se rekonstruiše posle reopen-a.

## Izvorni kod i licenca

Izvorni kod plugina i SVG ikonice su u [`crates/rf-ocs-plugin/`](crates/rf-ocs-plugin/). Ovaj plugin-only repozitorijum ne sadrži `rf-core` workspace potreban za rebuild; za instalaciju koristi gotov paket.

Adapter je označen kao **GPL-3.0-only**, u skladu sa OCS plugin API-jem.
