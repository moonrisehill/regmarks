//! # Repères de découpe — plug-in VectorCraft (WebAssembly, ABI v1, « object filter »)
//!
//! Autour de l'objet / du groupe sélectionné, le plug-in génère :
//!   * une **barre noire** de 3 mm, sur toute la largeur du visuel, alignée sur son bord inférieur ;
//!   * des **carrés noirs** de 3 mm, de part et d'autre du visuel, avec 3 mm de marge latérale :
//!       - H ≤ 300 mm  → 2 carrés par côté (haut et bas) ;
//!       - H > 300 mm  → carrés répartis à intervalles égaux, écart (centre à centre) ≤ 250 mm
//!                       et, quand c'est géométriquement possible, ≥ 200 mm.
//!
//! Le tout est livré comme UN seul objet (tracé composé « Repères de découpe »), ce qui garde le
//! panneau Calques propre. L'API de filtre v1 ne permet pas de créer de « groupe » : le tracé composé
//! en est l'équivalent le plus proche (voir le paramètre « Regrouper en un seul tracé composé »).
//!
//! ## Compilation
//! ```sh
//! rustup target add wasm32-unknown-unknown
//! cargo build --release --target wasm32-unknown-unknown
//! # → target/wasm32-unknown-unknown/release/vectorcraft_cut_marks.wasm
//! ```
//! Installation : Object › Plug-ins › Install Plug-in… (ou File › Open du .wasm).
//!
//! ## Unités
//! VectorCraft travaille en **points** (1 pt = 1/72 pouce, axe Y vers le bas).
//! Les paramètres de l'interface sont saisis en **millimètres** et convertis ici : 1 mm = 72 / 25,4 pt.

use serde::Deserialize;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------------------------

/// Points par millimètre (1 pouce = 25,4 mm = 72 pt).
const PT_PER_MM: f64 = 72.0 / 25.4;

/// Tolérance (en mm) pour les comparaisons de hauteur : absorbe les erreurs d'arrondi pt ↔ mm
/// (un visuel de « 300 mm » doit bien compter comme ≤ 300 mm).
const EPS_MM: f64 = 0.01;

/// Manifeste du plug-in (≤ 64 Kio). Les paramètres sont validés / bornés par l'hôte avant `vc_run`.
const MANIFEST: &str = r#"{
  "id": "fr.vectorcraft.cut-marks",
  "name": "Repères de découpe",
  "version": "1.0.0",
  "kind": "filter",
  "author": "Plug-in utilisateur",
  "description": "Ajoute une barre noire en bas du visuel sélectionné et des carrés de repérage de part et d'autre.",
  "params": {
    "barHeight": {
      "type": "number", "min": 0.1, "max": 100, "default": 3, "unit": "mm",
      "label": "Épaisseur de la barre", "group": "Barre",
      "help": "Barre noire sur toute la largeur du visuel, alignée sur son bord inférieur."
    },
    "markSize": {
      "type": "number", "min": 0.5, "max": 100, "default": 3, "unit": "mm",
      "label": "Côté des carrés", "group": "Carrés de repérage"
    },
    "markMargin": {
      "type": "number", "min": 0, "max": 200, "default": 3, "unit": "mm",
      "label": "Marge latérale", "group": "Carrés de repérage",
      "help": "Distance entre le visuel et chaque carré (bords gauche et droit)."
    },
    "centerOnEdges": {
      "type": "bool", "default": false,
      "label": "Centrer sur les bords haut et bas", "group": "Carrés de repérage",
      "help": "Désactivé : les carrés du haut et du bas sont alignés sur les bords du visuel (et sur la barre).\nActivé : ils sont centrés sur ces bords."
    },
    "singleRowMaxHeight": {
      "type": "number", "min": 10, "max": 100000, "default": 300, "unit": "mm",
      "label": "Hauteur max. pour 2 carrés par côté", "group": "Espacement",
      "help": "Jusqu'à cette hauteur : exactement 2 carrés par côté. Au-delà : répartition régulière."
    },
    "minSpacing": {
      "type": "number", "min": 10, "max": 100000, "default": 200, "unit": "mm",
      "label": "Espacement minimal", "group": "Espacement",
      "help": "Distance centre à centre entre deux carrés consécutifs. Respecté quand c'est possible."
    },
    "maxSpacing": {
      "type": "number", "min": 10, "max": 100000, "default": 250, "unit": "mm",
      "label": "Espacement maximal", "group": "Espacement",
      "help": "Distance centre à centre entre deux carrés consécutifs. Jamais dépassé."
    },
    "rgbBlack": {
      "type": "bool", "default": false,
      "label": "Noir RVB (au lieu de K 100 %)", "group": "Sortie",
      "help": "Par défaut le noir est du CMJN pur K = 100 %, adapté aux repères d'impression."
    },
    "compound": {
      "type": "bool", "default": true,
      "label": "Regrouper en un seul tracé composé", "group": "Sortie",
      "help": "Activé : un seul objet « Repères de découpe ».\nDésactivé : un objet par rectangle (à grouper ensuite avec Ctrl+G)."
    }
  }
}"#;

// ---------------------------------------------------------------------------------------------
// Types d'entrée (JSON envoyé par l'hôte)
// ---------------------------------------------------------------------------------------------

/// `input` : on ne lit que l'`id` des objets (pour les conserver) et leurs `bounds` (repli).
#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    objects: Vec<InObject>,
}

#[derive(Deserialize)]
struct InObject {
    /// Identifiant renvoyé tel quel : un objet absent de la sortie serait SUPPRIMÉ par l'hôte.
    id: Value,
    /// `[x0, y0, x1, y1]` en points.
    #[serde(default)]
    bounds: Option<[f64; 4]>,
}

/// `_context` ajouté aux paramètres par l'hôte.
#[derive(Deserialize, Default)]
struct Context {
    /// Boîte englobante géométrique de la sélection `[x0, y0, x1, y1]`, en points.
    bounds: Option<[f64; 4]>,
}

/// Paramètres de l'utilisateur (mm) — mêmes noms que dans le manifeste.
#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Params {
    bar_height: f64,
    mark_size: f64,
    mark_margin: f64,
    center_on_edges: bool,
    single_row_max_height: f64,
    min_spacing: f64,
    max_spacing: f64,
    rgb_black: bool,
    compound: bool,
    #[serde(rename = "_context")]
    context: Context,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            bar_height: 3.0,
            mark_size: 3.0,
            mark_margin: 3.0,
            center_on_edges: false,
            single_row_max_height: 300.0,
            min_spacing: 200.0,
            max_spacing: 250.0,
            rgb_black: false,
            compound: true,
            context: Context::default(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Géométrie
// ---------------------------------------------------------------------------------------------

/// Rectangle axis-aligned en points (y vers le bas).
#[derive(Clone, Copy)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    /// Sous-chemin fermé à 4 ancres sans poignées, toujours dans le même sens de rotation.
    fn subpath(&self) -> Value {
        json!({
            "closed": true,
            "anchors": [
                { "p": [self.x0, self.y0] },
                { "p": [self.x1, self.y0] },
                { "p": [self.x1, self.y1] },
                { "p": [self.x0, self.y1] }
            ]
        })
    }
}

/// Résultat du calcul de placement.
struct Layout {
    bar: Rect,
    marks: Vec<Rect>,
    /// Nombre de carrés par côté.
    per_side: usize,
    /// Écart effectif centre à centre entre deux carrés consécutifs (mm).
    pitch_mm: f64,
    /// `false` si l'écart effectif sort de [min, max] (cas géométriquement impossible, voir plus bas).
    in_range: bool,
}

fn bounds_are_valid(b: &[f64; 4]) -> bool {
    b.iter().all(|v| v.is_finite()) && b[2] > b[0] && b[3] > b[1]
}

/// Boîte englobante de la sélection : `_context.bounds` en priorité, sinon union des `bounds`
/// des objets reçus.
fn selection_bounds(ctx: &Context, objects: &[InObject]) -> Option<[f64; 4]> {
    if let Some(b) = ctx.bounds {
        if bounds_are_valid(&b) {
            return Some(b);
        }
    }
    let mut acc: Option<[f64; 4]> = None;
    for b in objects.iter().filter_map(|o| o.bounds) {
        if !b.iter().all(|v| v.is_finite()) {
            continue;
        }
        acc = Some(match acc {
            None => b,
            Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        });
    }
    acc.filter(bounds_are_valid)
}

/// Calcule la barre et les carrés pour la boîte `b = [x0, y0, x1, y1]` (points).
fn compute_layout(b: [f64; 4], p: &Params) -> Result<Layout, String> {
    let (x0, y0, x1, y1) = (b[0], b[1], b[2], b[3]);
    let mm = |v: f64| v * PT_PER_MM; // mm → pt

    let height_pt = y1 - y0;
    let height_mm = height_pt / PT_PER_MM;
    let bar_h = mm(p.bar_height);
    let s = mm(p.mark_size);
    let margin = mm(p.mark_margin);

    // Ordonnées (centres) du premier et du dernier carré.
    //  - alignés : le carré du haut touche le bord supérieur, celui du bas le bord inférieur
    //              (donc le bas du dernier carré est dans le prolongement du bas de la barre) ;
    //  - centrés : les centres sont exactement sur les bords haut et bas.
    let (c_first, c_last) = if p.center_on_edges {
        (y0, y1)
    } else {
        (y0 + s / 2.0, y1 - s / 2.0)
    };
    let span_pt = c_last - c_first; // distance centre à centre premier ↔ dernier carré

    if bar_h > height_pt || span_pt < s {
        return Err("Le visuel sélectionné est trop petit pour placer la barre et les repères.".into());
    }
    let span_mm = span_pt / PT_PER_MM;

    // Tolère min > max saisi à l'envers.
    let lo = p.min_spacing.min(p.max_spacing);
    let hi = p.min_spacing.max(p.max_spacing);

    // Nombre d'INTERVALLES entre le premier et le dernier carré (carrés par côté = intervalles + 1).
    let (gaps, check_range): (usize, bool) = if height_mm <= p.single_row_max_height + EPS_MM {
        // Cas 1 : H ≤ 300 mm → exactement 2 carrés par côté, aucune contrainte d'écart.
        (1, false)
    } else {
        // Cas 2 : on prend le plus petit nombre d'intervalles tel que écart ≤ max.
        // Si [min, max] est atteignable, ce choix respecte aussi l'écart minimal.
        // (Pour certaines hauteurs, p. ex. 300–400 mm, aucun entier ne tient dans [200, 250] :
        //  le MAXIMUM est alors prioritaire et l'écart effectif passe sous le minimum.)
        let n = ((span_mm / hi) - 1e-9).ceil().max(1.0) as usize;
        (n, true)
    };

    let pitch_pt = span_pt / gaps as f64;
    let pitch_mm = pitch_pt / PT_PER_MM;
    let in_range = !check_range || (pitch_mm >= lo - EPS_MM && pitch_mm <= hi + EPS_MM);

    // Colonnes gauche et droite, décalées de `margin` du visuel.
    let left_x = x0 - margin - s;
    let right_x = x1 + margin;

    let mut marks = Vec::with_capacity(2 * (gaps + 1));
    for i in 0..=gaps {
        let cy = if i == gaps { c_last } else { c_first + pitch_pt * i as f64 };
        let top = cy - s / 2.0;
        marks.push(Rect { x0: left_x, y0: top, x1: left_x + s, y1: top + s });
        marks.push(Rect { x0: right_x, y0: top, x1: right_x + s, y1: top + s });
    }

    // Barre : toute la largeur, bord inférieur aligné sur celui du visuel.
    let bar = Rect { x0, y0: y1 - bar_h, x1, y1 };

    Ok(Layout { bar, marks, per_side: gaps + 1, pitch_mm, in_range })
}

// ---------------------------------------------------------------------------------------------
// Construction de la sortie
// ---------------------------------------------------------------------------------------------

/// Remplissage noir uni : K 100 % (CMJN) par défaut, ou RVB (0, 0, 0).
fn black_fill(rgb: bool) -> Value {
    if rgb {
        json!({ "type": "solid", "color": { "model": "rgb", "r": 0.0, "g": 0.0, "b": 0.0 } })
    } else {
        json!({ "type": "solid", "color": { "model": "cmyk", "c": 0.0, "m": 0.0, "y": 0.0, "k": 1.0 } })
    }
}

/// Nouvel objet (sans `id`) : un seul rectangle, rempli, sans contour.
fn new_rect_object(name: &str, r: &Rect, fill: &Value) -> Value {
    json!({
        "type": "path",
        "name": name,
        "path": { "subpaths": [ r.subpath() ] },
        "fillRule": "nonzero",
        "fills": [ fill ],
        "opacity": 1.0
    })
}

/// Logique principale : JSON d'entrée + JSON de paramètres → JSON de sortie.
fn run(input: &[u8], params: &[u8]) -> Result<Value, String> {
    let input: Input =
        serde_json::from_slice(input).map_err(|e| format!("Entrée illisible : {e}"))?;
    let params: Params =
        serde_json::from_slice(params).map_err(|e| format!("Paramètres illisibles : {e}"))?;

    // Gestion d'erreur : rien de sélectionné → message affiché à l'utilisateur, document intact.
    let bounds = selection_bounds(&params.context, &input.objects).ok_or_else(|| {
        "Aucun objet sélectionné. Sélectionnez le visuel (objet ou groupe) avant de lancer le plug-in."
            .to_string()
    })?;

    let layout = compute_layout(bounds, &params)?;
    let fill = black_fill(params.rgb_black);

    // 1) On conserve TOUS les objets reçus : un objet absent de la sortie serait supprimé.
    let mut objects: Vec<Value> = input.objects.iter().map(|o| json!({ "id": o.id })).collect();

    // 2) On ajoute les nouveaux objets (sans `id`) : ils se placent juste au-dessus du dernier objet listé.
    if params.compound {
        // Un seul tracé composé : barre + carrés = 1 ligne dans le panneau Calques.
        let mut subpaths = vec![layout.bar.subpath()];
        subpaths.extend(layout.marks.iter().map(Rect::subpath));

        let name = format!(
            "Repères de découpe ({} carrés/côté, écart {:.1} mm{})",
            layout.per_side,
            layout.pitch_mm,
            if layout.in_range { "" } else { ", hors plage" }
        );
        objects.push(json!({
            "type": "compound",
            "name": name,
            "path": { "subpaths": subpaths },
            "fillRule": "nonzero",
            "fills": [ fill ],
            "opacity": 1.0
        }));
    } else {
        objects.push(new_rect_object("Barre de repérage", &layout.bar, &fill));
        for m in &layout.marks {
            objects.push(new_rect_object("Carré de repérage", m, &fill));
        }
    }

    Ok(json!({ "objects": objects }))
}

// ---------------------------------------------------------------------------------------------
// ABI v1 — points d'entrée exportés (aucun import, aucun WASI)
// ---------------------------------------------------------------------------------------------

/// Encode `(len << 32) | ptr` ; le tampon est « fuité » exprès (chaque exécution a une instance neuve).
fn pack(bytes: Vec<u8>) -> i64 {
    let len = bytes.len() as u64;
    let ptr = bytes.as_ptr() as usize as u64;
    std::mem::forget(bytes);
    ((len << 32) | ptr) as i64
}

#[no_mangle]
pub extern "C" fn vc_abi_version() -> i32 {
    1
}

#[no_mangle]
pub extern "C" fn vc_manifest() -> i64 {
    let len = MANIFEST.len() as u64;
    let ptr = MANIFEST.as_ptr() as usize as u64;
    ((len << 32) | ptr) as i64
}

/// Réserve `size` octets que l'hôte remplira (entrée et paramètres). Jamais libérés.
#[no_mangle]
pub extern "C" fn vc_alloc(size: i32) -> i32 {
    if size < 0 {
        return 0;
    }
    let mut buf: Vec<u8> = Vec::with_capacity((size as usize).max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as usize as i32
}

#[no_mangle]
pub extern "C" fn vc_run(input: i32, input_len: i32, params: i32, params_len: i32) -> i64 {
    if input_len < 0 || params_len < 0 {
        return -1;
    }
    // SAFETY : l'hôte a écrit ces plages via `vc_alloc` ; on ne fait que les lire.
    let (input, params) = unsafe {
        (
            std::slice::from_raw_parts(input as u32 as usize as *const u8, input_len as usize),
            std::slice::from_raw_parts(params as u32 as usize as *const u8, params_len as usize),
        )
    };

    let out = match run(input, params) {
        Ok(v) => v,
        // Un refus avec message : le document reste intact et l'utilisateur voit le texte.
        Err(msg) => json!({ "error": msg }),
    };

    match serde_json::to_vec(&out) {
        Ok(bytes) => pack(bytes),
        Err(_) => -2,
    }
}
