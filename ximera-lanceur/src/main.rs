// Lanceur Ximera pour Mac : met le jeu à jour depuis le serveur (seulement les fichiers changés), puis le lance
// avec ce Ruffle corrigé. Même fenêtre et même fonctionnement que Ximera.exe sous Windows (Lanceur.cs).
// Il reste ouvert (réduit) pendant le jeu : chaque minute il regarde si une nouvelle version est publiée
// et l'installe tout seul, une fois toutes les fenêtres de jeu fermées.
//
// Arguments de test : --test (met à jour puis se ferme, code de sortie 0 si tout va bien)
//                     --capture <image.png>, --capture-journal <image.png> (images de la fenêtre et du journal)
//                     --dossier <dossier> (dossier du joueur), --port-web <port> (serveur de fichiers d'essai)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod maj;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontFamily, FontId, Painter, Pos2, Rect, Stroke, StrokeKind};
use maj::{Lanceur, MiseAJour, Partage};

const FOND: Color32 = Color32::from_rgb(24, 20, 16);
const CADRE: Color32 = Color32::from_rgb(40, 33, 26);
const BORDURE: Color32 = Color32::from_rgb(90, 70, 45);
const OR: Color32 = Color32::from_rgb(255, 196, 64);
const TEXTE: Color32 = Color32::from_rgb(225, 215, 200);
const GRIS: Color32 = Color32::from_rgb(170, 160, 145);
const SURVOL: Color32 = Color32::from_rgb(62, 49, 36);
const ETEINT: Color32 = Color32::from_rgb(90, 80, 70);

const LIGNES: usize = 4;
const HAUT_LIGNE: f32 = 19.0;
const LARGEUR: f32 = 720.0;
const HAUTEUR: f32 = 440.0;

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

fn normal(taille: f32) -> FontId {
    FontId::new(taille, FontFamily::Proportional)
}

fn gras(taille: f32) -> FontId {
    FontId::new(taille, FontFamily::Name("gras".into()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode_test = false;
    let mut capture = None;
    let mut capture_journal = None;
    let mut dossier = None;
    let mut port_web = 8080u16;
    let mut i = 0;
    while i < args.len() {
        let suivant = args.get(i + 1).cloned();
        match args[i].as_str() {
            "--test" => mode_test = true,
            "--capture" => {
                capture = suivant.map(PathBuf::from);
                mode_test = true;
                i += 1;
            }
            "--capture-journal" => {
                capture_journal = suivant.map(PathBuf::from);
                mode_test = true;
                i += 1;
            }
            "--dossier" => {
                dossier = suivant.map(PathBuf::from);
                i += 1;
            }
            "--port-web" => {
                port_web = suivant.and_then(|s| s.parse().ok()).unwrap_or(8080);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let racine = dossier.unwrap_or_else(dossier_du_joueur);
    let _ = std::fs::create_dir_all(&racine);
    preparer(&racine, mode_test, &args);

    // Relancé après une mise à jour de lui-même : on garde le journal du lanceur précédent
    let mut journal = String::new();
    if std::fs::remove_file(racine.join("relance.txt")).is_ok() {
        journal = std::fs::read_to_string(racine.join("maj.log")).unwrap_or_default();
    }
    let l = Arc::new(Lanceur::new(racine, mode_test, args, port_web, journal));
    if !mode_test {
        let s = l.clone();
        std::thread::spawn(move || s.surveiller());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ximera")
            .with_inner_size([LARGEUR, HAUTEUR])
            .with_resizable(false)
            .with_maximize_button(false),
        centered: true,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Ximera",
        options,
        Box::new(move |cc| Ok(Box::new(Fenetre::new(cc, l, capture, capture_journal)))),
    );
}

// Mac : ~/Library/Application Support/Ximera (l'application elle-même peut être en lecture seule).
// Ailleurs (copie d'essai) : le dossier du programme, comme Ximera.exe.
fn dossier_du_joueur() -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join("Library").join("Application Support").join("Ximera");
        }
    }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

// Mac : serveur.txt livré dans l'application, puis passage de main à la copie à jour du lanceur si elle existe
fn preparer(racine: &std::path::Path, mode_test: bool, args: &[String]) {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
        let serveur = racine.join("serveur.txt");
        if !serveur.exists() {
            if let Some(livre) = exe.as_ref().and_then(|e| e.parent()).map(|d| d.join("../Resources/serveur.txt")) {
                let _ = std::fs::copy(livre, &serveur);
            }
        }
        let app = racine.join("mac/Ximera.app");
        let copie = app.join("Contents/MacOS/Ximera").canonicalize().ok();
        if !mode_test && copie.is_some() && exe != copie {
            let mut cmd = std::process::Command::new("/usr/bin/open");
            cmd.arg("-n").arg(&app);
            if !args.is_empty() {
                cmd.arg("--args").args(args);
            }
            if cmd.status().map(|s| s.success()).unwrap_or(false) {
                std::process::exit(0);
            }
        }
    }
    let _ = (racine, mode_test, args);
}

// Polices du système (Arial / Impact sur Mac, Segoe UI / Impact sous Windows), sinon celles d'egui
fn polices(ctx: &egui::Context) {
    let lire = |noms: &[&str]| -> Option<Vec<u8>> { noms.iter().find_map(|n| std::fs::read(n).ok()) };
    let mut defs = egui::FontDefinitions::default();
    let base = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut ajouter = |nom: &str, donnees: Option<Vec<u8>>| -> Vec<String> {
        let mut famille = Vec::new();
        if let Some(d) = donnees {
            defs.font_data.insert(nom.to_string(), Arc::new(egui::FontData::from_owned(d)));
            famille.push(nom.to_string());
        }
        famille
    };
    let n = ajouter(
        "normal",
        lire(&["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf", "C:\\Windows\\Fonts\\segoeui.ttf"]),
    );
    let g = ajouter(
        "gras",
        lire(&["/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/Library/Fonts/Arial Bold.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf"]),
    );
    let t = ajouter(
        "titre",
        lire(&["/System/Library/Fonts/Supplemental/Impact.ttf", "/Library/Fonts/Impact.ttf", "C:\\Windows\\Fonts\\impact.ttf"]),
    );
    let famille = |avant: Vec<String>| -> Vec<String> {
        let mut f = avant;
        f.extend(base.iter().cloned());
        f
    };
    let proportionnelle = famille(n.clone());
    let grasse = famille([g.clone(), n.clone()].concat());
    let titre = famille([t, g, n].concat());
    defs.families.insert(FontFamily::Proportional, proportionnelle);
    defs.families.insert(FontFamily::Name("gras".into()), grasse);
    defs.families.insert(FontFamily::Name("titre".into()), titre);
    ctx.set_fonts(defs);
}

struct Fenetre {
    l: Arc<Lanceur>,
    banniere: Option<egui::TextureHandle>,
    banniere_essai: f64,
    defilement: usize,
    molette: f32,
    journal: Option<usize>,
    journal_position: Option<Pos2>,
    journal_decalage: f32,
    journal_hauteur: f32,
    adresse: Option<String>,
    capture: Option<PathBuf>,
    capture_journal: Option<PathBuf>,
    test_etape: u8,
    test_images: u32,
}

impl Fenetre {
    fn new(cc: &eframe::CreationContext<'_>, l: Arc<Lanceur>, capture: Option<PathBuf>, capture_journal: Option<PathBuf>) -> Fenetre {
        polices(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        *l.ctx.lock().unwrap() = Some(cc.egui_ctx.clone());
        l.demarrer(false);
        Fenetre {
            l,
            banniere: None,
            banniere_essai: -10.0,
            defilement: 0,
            molette: 0.0,
            journal: None,
            journal_position: None,
            journal_decalage: 0.0,
            journal_hauteur: 0.0,
            adresse: None,
            capture,
            capture_journal,
            test_etape: 0,
            test_images: 0,
        }
    }

    // Bandeau illustré : l'image de chargement du jeu (téléchargée en premier)
    fn charger_banniere(&mut self, ctx: &egui::Context) {
        let t = ctx.input(|i| i.time);
        if self.banniere.is_some() || t - self.banniere_essai < 1.0 {
            return;
        }
        self.banniere_essai = t;
        if let Ok(img) = image::open(self.l.racine.join("jeu").join("loadingbanners").join("1.png")) {
            let img = img.to_rgba8();
            let taille = [img.width() as usize, img.height() as usize];
            let c = egui::ColorImage::from_rgba_unmultiplied(taille, img.as_raw());
            self.banniere = Some(ctx.load_texture("banniere", c, egui::TextureOptions::LINEAR));
        }
    }
}

fn ecrire(p: &Painter, s: &str, f: FontId, c: Color32, x: f32, y: f32) -> Rect {
    p.text(pos2(x + 2.0, y + 2.0), Align2::LEFT_TOP, s, f, c)
}

fn largeur(ctx: &egui::Context, s: &str, f: &FontId) -> f32 {
    ctx.fonts(|fo| fo.layout_no_wrap(s.to_string(), f.clone(), Color32::WHITE).size().x)
}

fn couper(ctx: &egui::Context, s: &str, f: &FontId, max: f32) -> String {
    if largeur(ctx, s, f) <= max {
        return s.to_string();
    }
    let mut c: Vec<char> = s.chars().collect();
    while c.len() > 1 && largeur(ctx, &(c.iter().collect::<String>() + "…"), f) > max {
        c.pop();
    }
    c.iter().collect::<String>() + "…"
}

fn encadre(p: &Painter, rect: Rect) {
    p.rect_filled(rect, 8.0, CADRE);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, BORDURE), StrokeKind::Inside);
}

fn souligner(p: &Painter, zone: Rect, c: Color32) {
    p.line_segment([pos2(zone.left(), zone.bottom() - 1.0), pos2(zone.right(), zone.bottom() - 1.0)], Stroke::new(1.0, c));
}

// Défilement à la molette ou au pavé tactile : un cran d'environ 40 points = une ligne
fn crans(cumul: &mut f32, delta: f32) -> i32 {
    *cumul += delta;
    let n = (*cumul / 40.0).trunc();
    *cumul -= n * 40.0;
    n as i32
}

fn enregistrer(image: &egui::ColorImage, chemin: &std::path::Path) {
    let octets: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let _ = image::save_buffer(chemin, &octets, image.size[0] as u32, image.size[1] as u32, image::ExtendedColorType::Rgba8);
}

fn capture_recue(ctx: &egui::Context) -> Option<Arc<egui::ColorImage>> {
    ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    })
}

impl eframe::App for Fenetre {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));
        let p: Partage = self.l.p.lock().unwrap().clone();
        if p.premier_plan {
            self.l.p.lock().unwrap().premier_plan = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.charger_banniere(ctx);

        egui::CentralPanel::default().frame(egui::Frame::default().fill(FOND)).show(ctx, |ui| {
            self.dessiner(ctx, ui.painter(), &p);
        });
        if !self.l.mode_test {
            self.interagir(ctx, &p);
        }
        self.fenetre_adresse(ctx);
        self.fenetre_journal(ctx, &p.nouveautes);
        if self.l.mode_test {
            self.test(ctx, &p);
        }
    }
}

impl Fenetre {
    fn dessiner(&mut self, ctx: &egui::Context, pt: &Painter, p: &Partage) {
        // Bandeau illustré, fondu vers le bas
        if let Some(b) = &self.banniere {
            let [w, h] = b.size();
            let h = h as f32 * LARGEUR / w as f32;
            pt.image(b.id(), r(0.0, -60.0, LARGEUR, h), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let mut fondu = egui::Mesh::default();
        fondu.colored_vertex(pos2(0.0, 120.0), Color32::TRANSPARENT);
        fondu.colored_vertex(pos2(LARGEUR, 120.0), Color32::TRANSPARENT);
        fondu.colored_vertex(pos2(LARGEUR, 231.0), FOND);
        fondu.colored_vertex(pos2(0.0, 231.0), FOND);
        fondu.add_triangle(0, 1, 2);
        fondu.add_triangle(0, 2, 3);
        pt.add(egui::Shape::mesh(fondu));
        pt.rect_filled(r(0.0, 230.0, LARGEUR, 210.0), 0.0, FOND);

        // Titre, avec un contour sombre
        let titre = FontId::new(74.0, FontFamily::Name("titre".into()));
        let contour = Color32::from_rgb(40, 22, 8);
        for k in 0..16 {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            ecrire(pt, "XIMERA", titre.clone(), contour, 24.0 + 3.0 * a.cos(), 104.0 + 3.0 * a.sin());
        }
        ecrire(pt, "XIMERA", titre, OR, 24.0, 104.0);
        ecrire(pt, "Serveur Dofus Rétro", gras(14.7), Color32::from_rgb(240, 225, 200), 32.0, 196.0);

        let titre_cadre = gras(14.0);
        let normale = normal(13.3);
        let petite = normal(12.7);

        // Nouveautés : une ligne par mise à jour, la plus récente en haut ; clic = détail
        let n = &p.nouveautes;
        let zone_nouveautes = r(20.0, 240.0, 430.0, 110.0);
        let zone_lignes = r(26.0, 270.0, 404.0, LIGNES as f32 * HAUT_LIGNE);
        let zone_barre = r(432.0, 270.0, 14.0, LIGNES as f32 * HAUT_LIGNE);
        encadre(pt, zone_nouveautes);
        ecrire(pt, "Nouveautés", titre_cadre.clone(), OR, 34.0, 248.0);
        self.defilement = self.defilement.min(n.len().saturating_sub(LIGNES));
        if n.is_empty() {
            ecrire(pt, if p.en_ligne == 0 { "…" } else { "Rien de neuf pour l'instant." }, petite.clone(), GRIS, 34.0, 272.0);
        } else {
            let f = normal(11.3);
            let l = largeur(ctx, "Clique pour le détail", &f);
            ecrire(pt, "Clique pour le détail", f, GRIS, zone_nouveautes.right() - 16.0 - l, 251.0);
        }
        let survol_ligne = self.ligne_sous(ctx, zone_lignes, n.len());
        for i in 0..LIGNES {
            let k = self.defilement + i;
            if k >= n.len() {
                break;
            }
            let m = &n[k];
            let y = zone_lignes.top() + i as f32 * HAUT_LIGNE;
            if survol_ligne == Some(k) && self.journal.is_none() {
                pt.rect_filled(r(zone_lignes.left(), y, zone_lignes.width(), HAUT_LIGNE - 1.0), 4.0, SURVOL);
                ecrire(pt, "›", normale.clone(), OR, zone_lignes.right() - 14.0, y - 2.0);
            }
            ecrire(pt, &m.date, petite.clone(), Color32::from_rgb(205, 165, 105), 32.0, y);
            ecrire(pt, &couper(ctx, &m.titre, &petite, 320.0), petite.clone(), TEXTE, if m.date.is_empty() { 32.0 } else { 80.0 }, y);
        }
        if n.len() > LIGNES {
            pt.rect_filled(r(zone_barre.left() + 5.0, zone_barre.top(), 4.0, zone_barre.height()), 2.0, Color32::from_rgb(60, 48, 36));
            let h = (zone_barre.height() * LIGNES as f32 / n.len() as f32).max(14.0);
            let y = zone_barre.top() + (zone_barre.height() - h) * self.defilement as f32 / (n.len() - LIGNES) as f32;
            pt.rect_filled(r(zone_barre.left() + 5.0, y, 4.0, h), 2.0, Color32::from_rgb(160, 120, 60));
        }

        // Serveur
        encadre(pt, r(466.0, 240.0, 234.0, 110.0));
        ecrire(pt, "Serveur", titre_cadre, OR, 480.0, 248.0);
        let voyant = match p.en_ligne {
            1 => Color32::from_rgb(80, 200, 90),
            -1 => Color32::from_rgb(220, 70, 60),
            _ => GRIS,
        };
        pt.circle_filled(pos2(488.0, 285.0), 6.0, voyant);
        let libelle = match p.en_ligne {
            1 => "En ligne",
            -1 => "Injoignable",
            _ => "Recherche…",
        };
        ecrire(pt, libelle, normale, TEXTE, 500.0, 275.0);
        if p.en_ligne == -1 && !p.travail {
            let z = ecrire(pt, "Changer l'adresse du serveur", petite.clone(), OR, 480.0, 322.0);
            if self.survole(ctx, z) && self.adresse.is_none() {
                souligner(pt, z, OR);
            }
        } else if !p.version.is_empty() {
            ecrire(pt, &format!("Version du jeu : {}", p.version), petite.clone(), GRIS, 482.0, 300.0);
        }

        // Progression
        let couleur = if p.erreur { Color32::from_rgb(240, 120, 100) } else { TEXTE };
        ecrire(pt, &couper(ctx, &p.etat, &petite, 500.0), petite, couleur, 20.0, 366.0);
        pt.rect_filled(r(20.0, 392.0, 500.0, 18.0), 9.0, Color32::from_rgb(55, 45, 35));
        let fr = if p.travail { p.progres } else if p.pret && !p.erreur { 1.0 } else { p.progres };
        if fr > 0.02 {
            pt.rect_filled(r(20.0, 392.0, 500.0 * fr.min(1.0) as f32, 18.0), 9.0, Color32::from_rgb(230, 150, 40));
        }

        // Bouton Jouer
        let zone_jouer = r(540.0, 368.0, 160.0, 52.0);
        let actif = p.pret && !p.travail;
        let survol = actif && self.survole(ctx, zone_jouer) && self.adresse.is_none() && self.journal.is_none();
        let fond = if !actif {
            Color32::from_rgb(95, 80, 65)
        } else if survol {
            Color32::from_rgb(248, 152, 50)
        } else {
            Color32::from_rgb(232, 132, 30)
        };
        pt.rect_filled(zone_jouer, 10.0, fond);
        let bord = if actif { Color32::from_rgb(255, 210, 120) } else { Color32::from_rgb(120, 105, 90) };
        pt.rect_stroke(zone_jouer, 10.0, Stroke::new(2.0, bord), StrokeKind::Inside);
        pt.text(
            zone_jouer.center() + vec2(0.0, 1.0),
            Align2::CENTER_CENTER,
            "JOUER",
            gras(21.3),
            if actif { Color32::WHITE } else { Color32::from_rgb(165, 150, 135) },
        );
    }

    fn souris(&self, ctx: &egui::Context) -> Option<Pos2> {
        ctx.input(|i| i.pointer.hover_pos())
    }

    fn survole(&self, ctx: &egui::Context, zone: Rect) -> bool {
        self.souris(ctx).map(|s| zone.contains(s)).unwrap_or(false)
    }

    fn ligne_sous(&self, ctx: &egui::Context, zone_lignes: Rect, nb: usize) -> Option<usize> {
        let s = self.souris(ctx)?;
        if !zone_lignes.contains(s) {
            return None;
        }
        let k = self.defilement + ((s.y - zone_lignes.top()) / HAUT_LIGNE) as usize;
        if k < nb {
            Some(k)
        } else {
            None
        }
    }

    fn interagir(&mut self, ctx: &egui::Context, p: &Partage) {
        if self.adresse.is_some() || self.journal.is_some() {
            return;
        }
        let n = p.nouveautes.len();
        let zone_nouveautes = r(20.0, 240.0, 430.0, 110.0);
        let zone_lignes = r(26.0, 270.0, 404.0, LIGNES as f32 * HAUT_LIGNE);
        let zone_barre = r(432.0, 270.0, 14.0, LIGNES as f32 * HAUT_LIGNE);
        let zone_jouer = r(540.0, 368.0, 160.0, 52.0);
        let zone_adresse = r(480.0, 322.0, 200.0, 18.0);
        let actif = p.pret && !p.travail;
        let lien = p.en_ligne == -1 && !p.travail;
        let ligne = self.ligne_sous(ctx, zone_lignes, n);
        let (clic, appui, molette) = ctx.input(|i| (i.pointer.primary_clicked(), i.pointer.primary_down(), i.raw_scroll_delta.y));

        if (actif && self.survole(ctx, zone_jouer)) || (lien && self.survole(ctx, zone_adresse)) || ligne.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        // Molette sur l'encadré Nouveautés
        if self.survole(ctx, zone_nouveautes) && molette != 0.0 {
            let c = crans(&mut self.molette, molette);
            let reste = n.saturating_sub(LIGNES) as i32;
            self.defilement = (self.defilement as i32 - c).clamp(0, reste.max(0)) as usize;
        }
        // Clic ou glisser sur la barre : la position verticale choisit la première mise à jour visible
        if appui && n > LIGNES {
            if let Some(s) = self.souris(ctx) {
                if zone_barre.contains(s) {
                    let reste = (n - LIGNES) as f32;
                    self.defilement = (((s.y - zone_barre.top()) / zone_barre.height() * reste).round()).clamp(0.0, reste) as usize;
                }
            }
        }
        if !clic {
            return;
        }
        if actif && self.survole(ctx, zone_jouer) {
            // Le lanceur se réduit au lieu de se fermer : il peut ouvrir d'autres fenêtres de jeu et installer les mises à jour
            match self.l.jouer() {
                Ok(true) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                Ok(false) => {}
                Err(err) => self.l.signaler(format!("Le jeu ne démarre pas : {}", err)),
            }
        } else if lien && self.survole(ctx, zone_adresse) {
            self.adresse = Some(p.serveur.clone());
        } else if let Some(k) = ligne {
            self.ouvrir_journal(ctx, k);
        }
    }

    fn ouvrir_journal(&mut self, ctx: &egui::Context, k: usize) {
        self.journal = Some(k);
        self.journal_decalage = 0.0;
        self.journal_position = ctx.input(|i| i.viewport().outer_rect).map(|r| r.center() - vec2(270.0, 230.0));
    }

    fn fenetre_adresse(&mut self, ctx: &egui::Context) {
        let Some(mut texte) = self.adresse.take() else { return };
        let mut fini = None;
        egui::Window::new("Adresse du serveur")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Adresse donnée par Florian :");
                let champ = ui.add(egui::TextEdit::singleline(&mut texte).desired_width(300.0));
                champ.request_focus();
                let entree = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let echap = ui.input(|i| i.key_pressed(egui::Key::Escape));
                ui.horizontal(|ui| {
                    if ui.button("Valider").clicked() || entree {
                        fini = Some(true);
                    }
                    if ui.button("Annuler").clicked() || echap {
                        fini = Some(false);
                    }
                });
            });
        match fini {
            Some(true) if !texte.trim().is_empty() => self.l.changer_adresse(&texte),
            Some(_) => {}
            None => self.adresse = Some(texte),
        }
    }

    // Fenêtre du journal : le détail d'une mise à jour, avec Plus ancienne / Plus récente (flèches du clavier aussi)
    fn fenetre_journal(&mut self, ctx: &egui::Context, liste: &[MiseAJour]) {
        let Some(mut index) = self.journal else { return };
        if index >= liste.len() {
            self.journal = None;
            return;
        }
        let mut b = egui::ViewportBuilder::default()
            .with_title("Journal des mises à jour")
            .with_inner_size([540.0, 424.0])
            .with_min_inner_size([540.0, 424.0])
            .with_resizable(true)
            .with_minimize_button(false);
        if let Some(pos) = self.journal_position {
            b = b.with_position(pos);
        }
        let mut fermer = false;
        let capture = if self.test_etape == 3 { self.capture_journal.clone() } else { None };
        let mut capture_faite = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("journal"), b, |ctx, _| {
            ctx.request_repaint_after(Duration::from_millis(100));
            let (fermeture, gauche, droite, clic, molette) = ctx.input(|i| {
                (
                    i.viewport().close_requested() || i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.pointer.primary_clicked(),
                    i.raw_scroll_delta.y,
                )
            });
            if fermeture {
                fermer = true;
            }
            let ecran = ctx.screen_rect();
            let (w, h) = (ecran.width(), ecran.height());
            let zone_ancienne = r(20.0, h - 36.0, 150.0, 22.0);
            let zone_recente = r(w - 170.0, h - 36.0, 150.0, 22.0);
            let souris = ctx.input(|i| i.pointer.hover_pos());
            let sur = |z: Rect| souris.map(|s| z.contains(s)).unwrap_or(false);
            let ancienne = index + 1 < liste.len();
            let recente = index > 0;
            let mut aller = |i: usize, index: &mut usize, decalage: &mut f32| {
                if i < liste.len() && i != *index {
                    *index = i;
                    *decalage = 0.0;
                }
            };
            if gauche || (clic && sur(zone_ancienne) && ancienne) {
                aller(index + 1, &mut index, &mut self.journal_decalage);
            }
            if (droite || (clic && sur(zone_recente))) && recente {
                aller(index - 1, &mut index, &mut self.journal_decalage);
            }
            if (sur(zone_ancienne) && index + 1 < liste.len()) || (sur(zone_recente) && index > 0) {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            let zone_detail = r(20.0, 78.0, w - 40.0, h - 128.0);
            let max = (self.journal_hauteur - (zone_detail.height() - 24.0)).max(0.0);
            self.journal_decalage = (self.journal_decalage - molette).clamp(0.0, max);

            egui::CentralPanel::default().frame(egui::Frame::default().fill(FOND)).show(ctx, |ui| {
                let pt = ui.painter();
                let m = &liste[index];
                let date = if m.date.is_empty() { "Mise à jour".to_string() } else { format!("Mise à jour du {}", m.date) };
                ecrire(pt, &date, normal(12.7), GRIS, 22.0, 14.0);
                ecrire(pt, &couper(ctx, &m.titre, &gras(18.7), w - 44.0), gras(18.7), OR, 20.0, 34.0);

                // Détail : une puce par ligne, coupé en plusieurs lignes si besoin
                encadre(pt, zone_detail);
                let clip = pt.with_clip_rect(zone_detail.shrink(1.0));
                let f = normal(13.3);
                let mut y = zone_detail.top() + 12.0 - self.journal_decalage;
                let debut = y;
                if m.details.is_empty() {
                    ecrire(&clip, "Pas de détail pour cette mise à jour.", f.clone(), GRIS, zone_detail.left() + 16.0, y);
                }
                for d in &m.details {
                    let largeur_texte = zone_detail.width() - 52.0;
                    let g = ctx.fonts(|fo| fo.layout(d.clone(), f.clone(), TEXTE, largeur_texte));
                    let hauteur = g.size().y;
                    ecrire(&clip, "•", f.clone(), OR, zone_detail.left() + 14.0, y - 2.0);
                    clip.galley(pos2(zone_detail.left() + 32.0, y), g, TEXTE);
                    y += hauteur + 7.0;
                }
                self.journal_hauteur = y - debut;
                // Détail plus long que l'encadré : flèches qui invitent à faire tourner la molette
                if self.journal_decalage > 0.0 {
                    ecrire(pt, "▲", normal(12.0), GRIS, zone_detail.right() - 22.0, zone_detail.top() + 2.0);
                }
                if self.journal_hauteur - self.journal_decalage > zone_detail.height() - 12.0 {
                    ecrire(pt, "▼", normal(12.0), GRIS, zone_detail.right() - 22.0, zone_detail.bottom() - 22.0);
                }

                // Navigation
                let fb = gras(13.3);
                let a = ecrire(pt, "← Plus ancienne", fb.clone(), if ancienne { OR } else { ETEINT }, zone_ancienne.left(), zone_ancienne.top());
                if sur(zone_ancienne) && ancienne {
                    souligner(pt, a, OR);
                }
                let texte_recente = "Plus récente →";
                let lr = largeur(ctx, texte_recente, &fb);
                let b = ecrire(pt, texte_recente, fb, if recente { OR } else { ETEINT }, zone_recente.right() - lr, zone_recente.top());
                if sur(zone_recente) && recente {
                    souligner(pt, b, OR);
                }
                let c = format!("{} sur {}", liste.len() - index, liste.len());
                let lc = largeur(ctx, &c, &normal(12.0));
                ecrire(pt, &c, normal(12.0), GRIS, (w - lc) / 2.0, zone_ancienne.top() + 2.0);
            });

            if let Some(chemin) = &capture {
                if let Some(img) = capture_recue(ctx) {
                    enregistrer(&img, chemin);
                    capture_faite = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
        });
        self.journal = if fermer { None } else { Some(index) };
        if capture_faite {
            self.test_etape = 4;
        }
    }

    // --test, --capture, --capture-journal : attend la fin de la mise à jour, enregistre les images puis se ferme
    fn test(&mut self, ctx: &egui::Context, p: &Partage) {
        match self.test_etape {
            0 => {
                if p.fini > 0 && !p.travail {
                    self.test_etape = 1;
                }
            }
            1 => {
                self.test_images += 1;
                if self.capture.is_none() {
                    self.test_etape = 2;
                } else if let Some(img) = capture_recue(ctx) {
                    enregistrer(&img, self.capture.as_ref().unwrap());
                    self.test_etape = 2;
                } else if self.test_images > 3 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
            2 => {
                if self.capture_journal.is_some() && !p.nouveautes.is_empty() {
                    self.ouvrir_journal(ctx, 0);
                    self.test_etape = 3;
                } else {
                    self.test_etape = 4;
                }
            }
            3 => {}
            _ => {
                self.l.ecrire_journal();
                let code = if p.pret && !p.erreur && p.en_ligne == 1 { 0 } else { 1 };
                std::process::exit(code);
            }
        }
        ctx.request_repaint();
    }
}
