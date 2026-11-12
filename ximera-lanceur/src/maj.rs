// Mise à jour du jeu depuis le serveur et lancement du jeu (même fonctionnement que Lanceur.cs sous Windows).
//
// Dossier du joueur (Mac : ~/Library/Application Support/Ximera) :
//   serveur.txt          adresse du serveur (modifiable par le joueur, jamais écrasée)
//   etat-maj.txt         empreintes des fichiers déjà vérifiés
//   manifeste.txt        copie de la dernière liste des fichiers du jeu
//   manifeste-mac.txt    copie de la dernière liste des fichiers propres au Mac
//   maj.log, jeu.log     journaux du dernier lancement et du dernier jeu lancé
//   jeu/                 le jeu (les mêmes fichiers que sous Windows)
//   mac/Ximera.app       copie à jour de ce lanceur (l'application téléchargée par le joueur passe la main à celle-ci)
//   mac/Jeu.app          Ruffle corrigé
//   Ximera.swf           petit film de démarrage, extrait de jeu/Dofus.exe
//   sauvegardes/         sauvegardes du jeu (le cache des textes ANKXTRASO* est effacé avant chaque partie)
// Sur le serveur (http://<adresse>:8080/client/) : version.txt, manifeste.txt.gz (fichiers jeu/ partagés avec Windows),
// manifeste-mac.txt.gz (fichiers mac/), nouveautes.txt, fichiers/<chemin> et colis/.
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

pub const PORT_CONNEXION: u16 = 4450;
pub const PORT_JEU: u16 = 5556;
const MINI_LISTE: usize = 100; // fichiers manquants à partir desquels on regarde les colis
const MINI_COLIS: usize = 20; // fichiers manquants dans un colis pour que le recevoir en vaille la peine
const SAUVEGARDE: Duration = Duration::from_secs(10);
pub const LANCEUR_MAC: &str = "mac/Ximera.app/Contents/MacOS/Ximera";
pub const BANNIERE: &str = "jeu/loadingbanners/1.png";

#[derive(Clone, Default)]
pub struct MiseAJour {
    pub date: String,
    pub titre: String,
    pub details: Vec<String>,
}

impl MiseAJour {
    // nouveautes.txt : « JJ/MM : titre » puis les détails « - … »
    pub fn lire(texte: &str) -> Vec<MiseAJour> {
        let mut l: Vec<MiseAJour> = Vec::new();
        for brut in texte.replace('\r', "").split('\n') {
            let x = brut.trim();
            if x.is_empty() || x.starts_with('#') {
                continue;
            }
            if let Some(d) = x.strip_prefix("- ") {
                if let Some(m) = l.last_mut() {
                    m.details.push(d.trim().to_string());
                    continue;
                }
            }
            let mut m = MiseAJour::default();
            match x.find(" : ") {
                Some(i) if i > 0 && x[..i].chars().count() <= 6 => {
                    m.date = x[..i].to_string();
                    m.titre = x[i + 3..].trim().to_string();
                }
                _ => m.titre = x.to_string(),
            }
            l.push(m);
        }
        l
    }
}

#[derive(Clone)]
struct Fichier {
    chemin: String,
    taille: u64,
    empreinte: String,
    date: i64,
}

// Ce que la fenêtre affiche (rempli par le fil de mise à jour)
#[derive(Clone)]
pub struct Partage {
    pub etat: String,
    pub travail: bool,
    pub pret: bool,
    pub erreur: bool,
    pub en_ligne: i32, // 0 inconnu, 1 en ligne, -1 injoignable
    pub a_jour: bool,
    pub verification: bool,
    pub maj_en_attente: bool,
    pub progres: f64,
    pub version: String,
    pub nouveautes: Vec<MiseAJour>,
    pub serveur: String,
    pub premier_plan: bool, // la fenêtre doit revenir au premier plan
    pub fini: u32,          // nombre de mises à jour terminées
}

struct Avancement {
    connus: HashMap<String, Fichier>,
    nouveau: HashMap<String, Fichier>,
    derniere: Instant,
}

pub struct Lanceur {
    pub racine: PathBuf,
    pub p: Mutex<Partage>,
    journal: Mutex<String>,
    jeux: Mutex<Vec<Child>>,
    avancement: Mutex<Option<Avancement>>,
    pub mode_test: bool,
    pub args: Vec<String>,
    pub port_web: u16,
    agent: ureq::Agent,
    pub ctx: Mutex<Option<eframe::egui::Context>>,
}

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

impl Lanceur {
    pub fn new(racine: PathBuf, mode_test: bool, args: Vec<String>, port_web: u16, journal: String) -> Lanceur {
        Lanceur {
            racine,
            p: Mutex::new(Partage {
                etat: "Recherche des mises à jour…".into(),
                travail: true,
                pret: false,
                erreur: false,
                en_ligne: 0,
                a_jour: false,
                verification: false,
                maj_en_attente: false,
                progres: 0.0,
                version: String::new(),
                nouveautes: Vec::new(),
                serveur: String::new(),
                premier_plan: false,
                fini: 0,
            }),
            journal: Mutex::new(journal),
            jeux: Mutex::new(Vec::new()),
            avancement: Mutex::new(None),
            mode_test,
            args,
            port_web,
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .build(),
            ctx: Mutex::new(None),
        }
    }

    fn maj<F: FnOnce(&mut Partage)>(&self, f: F) {
        f(&mut self.p.lock().unwrap());
        if let Some(c) = self.ctx.lock().unwrap().as_ref() {
            c.request_repaint();
        }
    }

    pub fn log(&self, ligne: &str) {
        let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut j = self.journal.lock().unwrap();
        j.push_str(&format!("{:02}:{:02}:{:02} {}\n", (s / 3600) % 24, (s / 60) % 60, s % 60, ligne));
    }

    pub fn ecrire_journal(&self) {
        let j = self.journal.lock().unwrap();
        let _ = fs::write(self.racine.join("maj.log"), j.as_bytes());
    }

    fn local(&self, chemin: &str) -> PathBuf {
        let mut p = self.racine.clone();
        for morceau in chemin.split('/') {
            p.push(morceau);
        }
        p
    }

    // ---------- Mise à jour (dans un fil à part pour garder la fenêtre vivante) ----------

    pub fn demarrer(self: &Arc<Self>, premier_plan: bool) {
        self.maj(|p| {
            p.travail = true;
            p.pret = false;
            p.erreur = false;
            p.en_ligne = 0;
            p.progres = 0.0;
            p.maj_en_attente = false;
            p.etat = "Recherche des mises à jour…".into();
            p.premier_plan = premier_plan;
        });
        let l = self.clone();
        std::thread::spawn(move || l.mettre_a_jour());
    }

    fn mettre_a_jour(self: &Arc<Self>) {
        let mut relancer = false;
        let ok = match self.executer(&mut relancer) {
            Ok(b) => b,
            Err(err) => {
                self.log(&format!("ERREUR {}", err));
                self.maj(|p| {
                    p.etat = format!("Mise à jour impossible : {}", err);
                    p.erreur = true;
                });
                let _ = self.sauver_avancement(true);
                false
            }
        };
        *self.avancement.lock().unwrap() = None;
        let manque = self.manque();
        let pret = manque.is_none();
        self.maj(|p| {
            p.a_jour = ok;
            p.pret = pret;
            if pret && !ok && !p.erreur {
                p.etat = "Serveur injoignable : le jeu n'a pas pu être mis à jour".into();
            }
            if !pret && !p.erreur {
                p.etat = if ok {
                    format!("Jeu incomplet : {} manque", manque.clone().unwrap_or_default())
                } else {
                    "Serveur injoignable : impossible d'installer le jeu".into()
                };
            }
            p.travail = false;
            p.fini += 1;
        });
        let etat = self.p.lock().unwrap().etat.clone();
        self.log(&format!("Fin : {}", etat));
        self.ecrire_journal();
        if ok && relancer && !self.mode_test {
            self.relancer();
        }
    }

    // Ce qu'il faut pour jouer (None si tout est là)
    fn manque(&self) -> Option<String> {
        if !self.local("jeu/loader.swf").exists() {
            return Some("jeu/loader.swf".into());
        }
        if !self.racine.join("Ximera.swf").exists() {
            return Some("Ximera.swf".into());
        }
        if !self.ruffle().exists() {
            return Some(self.ruffle().display().to_string());
        }
        None
    }

    pub fn ruffle(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.local("mac/Jeu.app/Contents/MacOS/ruffle")
        } else {
            // Copie d'essai sous Windows : Ruffle posé à la main dans ruffle/
            self.local("ruffle/ruffle_desktop.exe")
        }
    }

    // Nouveau lanceur reçu (ou premier lancement depuis l'application téléchargée) : on passe la main à la copie à jour
    #[cfg(target_os = "macos")]
    fn relancer(&self) {
        let app = self.local("mac/Ximera.app");
        self.log("Nouveau lanceur : redemarrage");
        self.ecrire_journal();
        let _ = fs::write(self.racine.join("relance.txt"), b"1");
        let mut cmd = Command::new("/usr/bin/open");
        cmd.arg("-n").arg(&app);
        if !self.args.is_empty() {
            cmd.arg("--args").args(&self.args);
        }
        if cmd.status().map(|s| s.success()).unwrap_or(false) {
            std::process::exit(0);
        }
        let _ = fs::remove_file(self.racine.join("relance.txt"));
    }

    #[cfg(not(target_os = "macos"))]
    fn relancer(&self) {}

    fn telecharger(&self, url: &str) -> R<Vec<u8>> {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let rep = self.agent.get(&format!("{}?t={}", url, t)).call().map_err(e)?;
        let mut v = Vec::new();
        rep.into_reader().read_to_end(&mut v).map_err(e)?;
        Ok(v)
    }

    fn texte(&self, url: &str) -> R<String> {
        Ok(String::from_utf8_lossy(&self.telecharger(url)?).into_owned())
    }

    // Liste des fichiers : retéléchargée seulement si le numéro de version a changé
    fn manifeste(&self, base: &str, nom: &str, v: &str) -> R<String> {
        let copie = self.racine.join(format!("{}.txt", nom));
        let entete = format!("version {}\n", v);
        let texte = fs::read_to_string(&copie).unwrap_or_default();
        if texte.starts_with(&entete) {
            return Ok(texte);
        }
        let gz = self.telecharger(&format!("{}{}.txt.gz", base, nom))?;
        let mut texte = String::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut texte).map_err(e)?;
        if !texte.starts_with(&entete) {
            return Err("liste des fichiers en cours de publication, réessaie dans une minute".into());
        }
        fs::write(&copie, texte.as_bytes()).map_err(e)?;
        Ok(texte)
    }

    // Renvoie vrai si le jeu est à jour (faux si le serveur est injoignable)
    fn executer(self: &Arc<Self>, relancer: &mut bool) -> R<bool> {
        self.log(&format!("Lanceur {}", self.racine.display()));
        let serveur = fs::read_to_string(self.racine.join("serveur.txt")).map(|s| s.trim().to_string()).unwrap_or_default();
        self.maj(|p| p.serveur = serveur.clone());
        if serveur.is_empty() {
            self.maj(|p| {
                p.en_ligne = -1;
                p.etat = "Adresse du serveur inconnue".into();
            });
            return Ok(false);
        }
        self.log(&format!("Serveur {}", serveur));
        let ouvert = port_ouvert(&serveur, PORT_CONNEXION);
        self.maj(|p| p.en_ligne = if ouvert { 1 } else { -1 });
        self.log(&format!("Connexion {} : {}", PORT_CONNEXION, if ouvert { "ouverte" } else { "fermee" }));
        let base = format!("http://{}:{}/client/", serveur, self.port_web);

        let v = match self.texte(&format!("{}version.txt", base)) {
            Ok(t) => t.trim().to_string(),
            Err(err) => {
                self.log(&format!("Version : {}", err));
                return Ok(false);
            }
        };
        let principal = match self.manifeste(&base, "manifeste", &v) {
            Ok(t) => t,
            Err(err) if err.contains("publication") => return Err(err),
            Err(err) => {
                self.log(&format!("Liste des fichiers : {}", err));
                return Ok(false);
            }
        };
        let mac = match self.manifeste(&base, "manifeste-mac", &v) {
            Ok(t) => t,
            Err(err) if err.contains("404") => return Err("le jeu pour Mac n'est pas encore publié sur le serveur".into()),
            Err(err) => return Err(err),
        };
        if let Ok(t) = self.texte(&format!("{}nouveautes.txt", base)) {
            let n = MiseAJour::lire(&t);
            self.maj(|p| p.nouveautes = n);
        }

        // Fichiers du jeu (partagés avec Windows) + fichiers propres au Mac
        let mut liste: Vec<Fichier> = Vec::new();
        for (texte, prefixe) in [(&principal, "jeu/"), (&mac, "mac/")] {
            for ligne in texte.replace('\r', "").split('\n') {
                if let Some(x) = ligne.strip_prefix("version ") {
                    let x = x.trim().to_string();
                    self.maj(|p| p.version = x);
                    continue;
                }
                let p: Vec<&str> = ligne.split('\t').collect();
                if p.len() != 3 || !p[2].starts_with(prefixe) {
                    continue;
                }
                if p[2].contains("..") || p[2].starts_with('/') || p[2].contains('\\') {
                    return Err("liste des fichiers abîmée".into());
                }
                liste.push(Fichier { empreinte: p[0].to_string(), taille: p[1].parse().map_err(e)?, chemin: p[2].to_string(), date: 0 });
            }
        }
        if liste.is_empty() {
            return Err("liste des fichiers vide".into());
        }
        self.log(&format!("Version {} : {} fichiers", v, liste.len()));

        // 1. Ce qui est déjà bon (état connu, sinon relecture du fichier)
        let connus = self.lire_etat();
        *self.avancement.lock().unwrap() = Some(Avancement { connus: connus.clone(), nouveau: HashMap::new(), derniere: Instant::now() });
        let mut a_telecharger: Vec<Fichier> = Vec::new();
        let nb = liste.len();
        for (i, f) in liste.iter().enumerate() {
            let local = self.local(&f.chemin);
            if let Ok(m) = fs::metadata(&local) {
                if m.is_file() && m.len() == f.taille {
                    let d = date(&local);
                    let mut bon = connus.get(&f.chemin).map(|c| c.taille == f.taille && c.date == d && c.empreinte == f.empreinte).unwrap_or(false);
                    if !bon {
                        self.maj(|p| {
                            p.etat = format!("Vérification des fichiers : {} sur {}", i + 1, nb);
                            p.progres = (i + 1) as f64 / nb as f64;
                        });
                        bon = empreinte_fichier(&local).map(|h| h == f.empreinte).unwrap_or(false);
                    }
                    if bon {
                        self.noter(f, d);
                        self.sauver_avancement(false)?;
                        continue;
                    }
                }
            }
            a_telecharger.push(f.clone());
        }

        // 2. Téléchargement de ce qui manque ou a changé
        let total: u64 = a_telecharger.iter().map(|f| f.taille).sum();
        let nb_fichiers = a_telecharger.len();
        let mut fait: u64 = 0;
        let mut recus: usize = 0;
        self.log(&format!("A telecharger : {} fichiers, {} octets", nb_fichiers, total));
        if a_telecharger.iter().any(|f| f.chemin == LANCEUR_MAC) || !lance_depuis_copie(self) {
            *relancer = true;
        }
        // L'image du bandeau d'abord : la fenêtre l'affiche dès qu'elle arrive
        if let Some(i) = a_telecharger.iter().position(|f| f.chemin == BANNIERE) {
            let f = a_telecharger.remove(i);
            recus += 1;
            self.recevoir(&base, &f, recus, nb_fichiers, total, &mut fait)?;
        }
        // Beaucoup de fichiers manquants (première installation) : d'abord les colis, le reste un par un
        if a_telecharger.len() >= MINI_LISTE {
            if let Err(err) = self.recevoir_colis(&base, &a_telecharger, nb_fichiers, total, &mut fait, &mut recus) {
                self.log(&format!("Colis abandonnes : {}", err));
            }
            let fait_colis: Vec<String> = {
                let a = self.avancement.lock().unwrap();
                a.as_ref().map(|a| a.nouveau.keys().cloned().collect()).unwrap_or_default()
            };
            a_telecharger.retain(|f| !fait_colis.contains(&f.chemin));
            self.log(&format!("Restent a telecharger : {} fichiers", a_telecharger.len()));
        }
        for f in &a_telecharger {
            recus += 1;
            self.recevoir(&base, f, recus, nb_fichiers, total, &mut fait)?;
        }

        // 3. Fichiers retirés du jeu : seulement ceux que le lanceur avait installés
        let presents: std::collections::HashSet<&str> = liste.iter().map(|f| f.chemin.as_str()).collect();
        for chemin in connus.keys() {
            if presents.contains(chemin.as_str()) || chemin.contains("..") {
                continue;
            }
            if fs::remove_file(self.local(chemin)).is_ok() {
                self.log(&format!("Retire {}", chemin));
            }
        }
        let nouveau = self.avancement.lock().unwrap().take().map(|a| a.nouveau).unwrap_or_default();
        self.ecrire_etat(&nouveau)?;
        self.ecrire_config(&serveur)?;
        self.extraire_demarrage()?;

        self.maj(|p| {
            p.progres = 1.0;
            p.etat = if nb_fichiers == 0 { "Le jeu est à jour !".into() } else { "Mise à jour terminée !".into() };
        });
        Ok(true)
    }

    fn noter(&self, f: &Fichier, d: i64) {
        if let Some(a) = self.avancement.lock().unwrap().as_mut() {
            a.nouveau.insert(f.chemin.clone(), Fichier { date: d, ..f.clone() });
        }
    }

    fn afficher_avancement(&self, i: usize, nb: usize, fait: u64, total: u64) {
        self.maj(|p| {
            p.etat = format!(
                "Mise à jour : {} fichier{} sur {}  ({} sur {})",
                i,
                if i > 1 { "s" } else { "" },
                nb,
                mo(fait),
                mo(total)
            );
            if total > 0 {
                p.progres = fait as f64 / total as f64;
            }
        });
    }

    // Télécharge un fichier (4 essais), le vérifie et le met en place
    fn recevoir(&self, base: &str, f: &Fichier, i: usize, nb: usize, total: u64, fait: &mut u64) -> R<()> {
        self.afficher_avancement(i, nb, *fait, total);
        let url = format!("{}fichiers/{}?v={}", base, url_chemin(&f.chemin), &f.empreinte[..12.min(f.empreinte.len())]);
        let cible = self.local(&f.chemin);
        let avant = *fait;
        let mut essai = 1;
        loop {
            match self.telecharger_fichier(&url, &cible, f, total, fait) {
                Ok(()) => break,
                Err(err) => {
                    self.log(&format!("Essai {} {} : {}", essai, f.chemin, err));
                    *fait = avant;
                    if essai == 4 || err.contains("ferme le jeu") {
                        return Err(format!("téléchargement de {} impossible ({})", f.chemin, err));
                    }
                    std::thread::sleep(Duration::from_millis(2000 * essai));
                    essai += 1;
                }
            }
        }
        self.noter(f, date(&cible));
        self.log(&format!("Recu {}", f.chemin));
        self.sauver_avancement(false)
    }

    fn telecharger_fichier(&self, url: &str, cible: &Path, f: &Fichier, total: u64, fait: &mut u64) -> R<()> {
        if let Some(d) = cible.parent() {
            fs::create_dir_all(d).map_err(e)?;
        }
        let part = part(cible);
        let rep = self.agent.get(url).call().map_err(e)?;
        let mut entree = rep.into_reader();
        let mut sortie = fs::File::create(&part).map_err(e)?;
        let mut sha = Sha256::new();
        let mut buf = vec![0u8; 65536];
        let mut dernier = Instant::now();
        loop {
            let n = entree.read(&mut buf).map_err(e)?;
            if n == 0 {
                break;
            }
            sortie.write_all(&buf[..n]).map_err(e)?;
            sha.update(&buf[..n]);
            *fait += n as u64;
            if total > 0 && dernier.elapsed() > Duration::from_millis(100) {
                dernier = Instant::now();
                let fr = *fait as f64 / total as f64;
                self.maj(|p| p.progres = fr);
            }
        }
        drop(sortie);
        if hex(&sha.finalize()) != f.empreinte {
            let _ = fs::remove_file(&part);
            return Err("fichier abîmé pendant le téléchargement".into());
        }
        remplacer(&part, cible, &f.chemin)
    }

    // Reçoit les colis qui contiennent assez de fichiers manquants. Chaque fichier est vérifié et installé tout de suite :
    // après une coupure, ou si le serveur n'a pas de colis, le reste se télécharge un par un.
    fn recevoir_colis(&self, base: &str, a_telecharger: &[Fichier], nb_fichiers: usize, total: u64, fait: &mut u64, recus: &mut usize) -> R<()> {
        let mut manquants: HashMap<String, Fichier> = a_telecharger.iter().map(|f| (f.chemin.clone(), f.clone())).collect();
        let gz = self.telecharger(&format!("{}colis/liste.txt.gz", base))?;
        let mut texte = String::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut texte).map_err(e)?;
        let mut colis: Vec<(String, Vec<Fichier>)> = Vec::new();
        for ligne in texte.split('\n') {
            let p: Vec<&str> = ligne.split('\t').collect();
            if p.len() != 3 {
                continue;
            }
            if p[0] == "colis" {
                colis.push((p[1].to_string(), Vec::new()));
                continue;
            }
            if let Some(c) = colis.last_mut() {
                c.1.push(Fichier { empreinte: p[0].to_string(), taille: p[1].parse().map_err(e)?, chemin: p[2].to_string(), date: 0 });
            }
        }
        for (nom, contenu) in &colis {
            let voulus = contenu.iter().filter(|f| voulu(f, &manquants)).count();
            if voulus < MINI_COLIS {
                continue;
            }
            let mut essai = 1;
            loop {
                match self.lire_colis(&format!("{}colis/{}", base, nom), contenu, &mut manquants, nb_fichiers, total, fait, recus) {
                    Ok(()) => break,
                    Err(err) => {
                        self.log(&format!("Essai {} colis {} : {}", essai, nom, err));
                        if essai == 2 {
                            return Err(err);
                        }
                        std::thread::sleep(Duration::from_secs(2));
                        essai += 1;
                    }
                }
            }
            self.log(&format!("Colis {} recu", nom));
            self.sauver_avancement(false)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lire_colis(&self, url: &str, contenu: &[Fichier], manquants: &mut HashMap<String, Fichier>, nb: usize, total: u64, fait: &mut u64, recus: &mut usize) -> R<()> {
        let rep = self.agent.get(url).call().map_err(e)?;
        let mut entree = rep.into_reader();
        let mut buf: Vec<u8> = Vec::new();
        for f in contenu {
            let taille = f.taille as usize;
            buf.resize(taille, 0);
            entree.read_exact(&mut buf).map_err(|_| "colis coupé".to_string())?;
            if !voulu(f, manquants) {
                continue;
            }
            // Fichier abîmé : on le laisse dans la liste, il sera téléchargé seul
            if hex(&Sha256::digest(&buf)) != f.empreinte {
                self.log(&format!("Abime dans le colis : {}", f.chemin));
                continue;
            }
            let cible = self.local(&f.chemin);
            if let Some(d) = cible.parent() {
                fs::create_dir_all(d).map_err(e)?;
            }
            let part = part(&cible);
            fs::write(&part, &buf).map_err(e)?;
            remplacer(&part, &cible, &f.chemin)?;
            self.noter(f, date(&cible));
            manquants.remove(&f.chemin);
            *fait += taille as u64;
            *recus += 1;
            self.afficher_avancement(*recus, nb, *fait, total);
        }
        Ok(())
    }

    // config.xml du jeu : modèle publié + adresse de serveur.txt (réécrit seulement s'il change)
    fn ecrire_config(&self, serveur: &str) -> R<()> {
        let modele = match fs::read_to_string(self.local("jeu/config.modele.xml")) {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };
        let contenu = modele.replace("{SERVEUR}", serveur);
        let config = self.local("jeu/config.xml");
        if fs::read_to_string(&config).ok().as_deref() != Some(contenu.as_str()) {
            fs::write(&config, contenu).map_err(e)?;
        }
        Ok(())
    }

    // Petit film de démarrage, collé à la fin de Dofus.exe (lecteur Flash Windows) : 8 derniers octets = 56 34 12 FA + taille
    fn extraire_demarrage(&self) -> R<()> {
        let exe = match fs::read(self.local("jeu/Dofus.exe")) {
            Ok(d) => d,
            Err(_) => return Ok(()),
        };
        let n = exe.len();
        if n < 8 || exe[n - 8..n - 4] != [0x56, 0x34, 0x12, 0xFA] {
            return Err("film de démarrage introuvable dans Dofus.exe".into());
        }
        let taille = u32::from_le_bytes([exe[n - 4], exe[n - 3], exe[n - 2], exe[n - 1]]) as usize;
        if taille + 8 > n {
            return Err("film de démarrage abîmé".into());
        }
        let film = &exe[n - 8 - taille..n - 8];
        let cible = self.racine.join("Ximera.swf");
        if fs::read(&cible).ok().as_deref() != Some(film) {
            fs::write(&cible, film).map_err(e)?;
        }
        Ok(())
    }

    fn lire_etat(&self) -> HashMap<String, Fichier> {
        let mut d = HashMap::new();
        if let Ok(t) = fs::read_to_string(self.racine.join("etat-maj.txt")) {
            for ligne in t.lines() {
                let p: Vec<&str> = ligne.split('\t').collect();
                if p.len() != 4 {
                    continue;
                }
                if let (Ok(taille), Ok(date)) = (p[1].parse(), p[2].parse()) {
                    d.insert(p[3].to_string(), Fichier { empreinte: p[0].to_string(), taille, date, chemin: p[3].to_string() });
                }
            }
        }
        d
    }

    fn ecrire_etat(&self, d: &HashMap<String, Fichier>) -> R<()> {
        let mut s = String::new();
        for f in d.values() {
            s.push_str(&format!("{}\t{}\t{}\t{}\n", f.empreinte, f.taille, f.date, f.chemin));
        }
        let chemin = self.racine.join("etat-maj.txt");
        let tmp = self.racine.join("etat-maj.txt.tmp");
        fs::write(&tmp, s).map_err(e)?;
        fs::rename(&tmp, &chemin).map_err(e)
    }

    // Enregistre ce qui est déjà fait, pour qu'une mise à jour coupée reprenne sans tout relire
    fn sauver_avancement(&self, maintenant: bool) -> R<()> {
        let d = {
            let mut a = self.avancement.lock().unwrap();
            let a = match a.as_mut() {
                Some(a) => a,
                None => return Ok(()),
            };
            if !maintenant && a.derniere.elapsed() < SAUVEGARDE {
                return Ok(());
            }
            a.derniere = Instant::now();
            let mut d = a.connus.clone();
            for (k, v) in &a.nouveau {
                d.insert(k.clone(), v.clone());
            }
            d
        };
        self.ecrire_etat(&d)
    }

    // ---------- Surveillance pendant le jeu ----------

    // Toutes les 5 s : une mise à jour en attente s'installe dès que le jeu est fermé.
    // Toutes les minutes : une nouvelle version est-elle publiée ? (ou nouvel essai si la dernière mise à jour a échoué)
    pub fn surveiller(self: Arc<Self>) {
        let mut tours = 0;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let p = self.p.lock().unwrap().clone();
            if p.travail || p.verification {
                continue;
            }
            if p.maj_en_attente {
                if !self.jeu_ouvert() {
                    self.demarrer(true);
                }
                continue;
            }
            tours += 1;
            if tours < 12 {
                continue;
            }
            tours = 0;
            self.maj(|p| p.verification = true);
            let mut nouvelle = false;
            if !p.serveur.is_empty() {
                let ouvert = port_ouvert(&p.serveur, PORT_CONNEXION);
                self.maj(|p| p.en_ligne = if ouvert { 1 } else { -1 });
                if let Ok(v) = self.texte(&format!("http://{}:{}/client/version.txt", p.serveur, self.port_web)) {
                    nouvelle = !p.a_jour || v.trim() != p.version;
                }
            }
            self.maj(|p| p.verification = false);
            if nouvelle {
                if self.jeu_ouvert() {
                    self.maj(|p| {
                        p.maj_en_attente = true;
                        p.etat = "Nouvelle mise à jour : elle s'installera dès que tu fermes le jeu".into();
                    });
                } else {
                    self.demarrer(true);
                }
            }
        }
    }

    // Vrai si un jeu lancé depuis ce dossier tourne encore
    pub fn jeu_ouvert(&self) -> bool {
        let mut j = self.jeux.lock().unwrap();
        j.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        if !j.is_empty() {
            return true;
        }
        drop(j);
        #[cfg(target_os = "macos")]
        {
            // Jeu lancé par un lanceur précédent (fermé puis rouvert pendant la partie)
            if let Ok(s) = Command::new("/usr/bin/pgrep").arg("-f").arg(self.ruffle()).stdout(Stdio::null()).status() {
                return s.success();
            }
        }
        false
    }

    // Lance une fenêtre de jeu (Ruffle corrigé, avec le petit film de démarrage et les réglages vérifiés sur le PC)
    pub fn jouer(self: &Arc<Self>) -> R<bool> {
        let (attente, serveur) = {
            let p = self.p.lock().unwrap();
            (p.maj_en_attente, p.serveur.clone())
        };
        let ouvert = self.jeu_ouvert();
        if attente && !ouvert {
            self.demarrer(true);
            return Ok(false);
        }
        let sauvegardes = self.racine.join("sauvegardes");
        if !ouvert {
            // Le cache des textes fige le jeu à la connexion sous Ruffle : effacé, les textes se retéléchargent
            let n = effacer_cache_textes(&sauvegardes);
            self.log(&format!("Cache des textes efface : {} fichiers", n));
        }
        let jeu = self.racine.join("jeu");
        let journal = if ouvert { None } else { fs::File::create(self.racine.join("jeu.log")).ok() };
        let mut cmd = Command::new(self.ruffle());
        cmd.arg(self.racine.join("Ximera.swf"))
            .arg("--base")
            .arg(url_dossier(&jeu))
            .args(["--no-gui", "--width", "1024", "--height", "768"])
            .args(["--tcp-connections", "allow"])
            .arg("--socket-allow")
            .arg(format!("{}:{}", serveur, PORT_CONNEXION))
            .arg("--socket-allow")
            .arg(format!("{}:{}", serveur, PORT_JEU))
            .args(["--filesystem-access-mode", "allow", "--open-url-mode", "deny", "--storage", "disk"])
            .arg("--save-directory")
            .arg(&sauvegardes)
            .arg("--config")
            .arg(self.racine.join("ruffle-reglages"))
            .arg("--cache-directory")
            .arg(self.racine.join("ruffle-cache"))
            .args(["--player-version", "32"])
            .current_dir(&jeu)
            .env("RUST_LOG", "error")
            .stdin(Stdio::null());
        match journal {
            Some(f) => {
                let f2 = f.try_clone().map_err(e)?;
                cmd.stdout(f).stderr(f2);
            }
            None => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        let enfant = cmd.spawn().map_err(e)?;
        self.log("Jeu lance");
        self.jeux.lock().unwrap().push(enfant);
        if !attente {
            self.maj(|p| p.etat = "Jeu lancé ! Le lanceur reste là pour les prochaines mises à jour".into());
        }
        Ok(true)
    }

    pub fn changer_adresse(self: &Arc<Self>, adresse: &str) {
        let _ = fs::write(self.racine.join("serveur.txt"), adresse.trim());
        self.demarrer(false);
    }

    pub fn signaler(&self, message: String) {
        self.maj(|p| {
            p.etat = message;
            p.erreur = true;
        });
    }
}

// Vrai si ce lanceur est la copie à jour du dossier du joueur (toujours vrai hors Mac)
fn lance_depuis_copie(l: &Lanceur) -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    let actuel = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let copie = l.local(LANCEUR_MAC).canonicalize().ok();
    actuel.is_some() && actuel == copie
}

fn voulu(f: &Fichier, manquants: &HashMap<String, Fichier>) -> bool {
    manquants.get(&f.chemin).map(|m| m.empreinte == f.empreinte && m.taille == f.taille).unwrap_or(false)
}

fn part(cible: &Path) -> PathBuf {
    let mut s = cible.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn remplacer(part: &Path, cible: &Path, chemin: &str) -> R<()> {
    if let Err(err) = fs::rename(part, cible) {
        let _ = fs::remove_file(part);
        return Err(if cfg!(windows) { "ferme le jeu avant la mise à jour".to_string() } else { err.to_string() });
    }
    // Programmes de l'application (lanceur, Ruffle) : exécutables
    #[cfg(unix)]
    {
        if chemin.contains("/Contents/MacOS/") {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(cible, fs::Permissions::from_mode(0o755));
        }
    }
    let _ = chemin;
    Ok(())
}

fn date(p: &Path) -> i64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn empreinte_fichier(p: &Path) -> R<String> {
    let mut f = fs::File::open(p).map_err(e)?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(e)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(hex(&sha.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn mo(octets: u64) -> String {
    format!("{:.1} Mo", octets as f64 / 1048576.0).replace('.', ",")
}

fn encoder(s: &str, garder: &[u8]) -> String {
    let mut r = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || garder.contains(&b) {
            r.push(b as char);
        } else {
            r.push_str(&format!("%{:02X}", b));
        }
    }
    r
}

fn url_chemin(chemin: &str) -> String {
    encoder(chemin, b"/")
}

// Adresse file:// d'un dossier (le jeu y cherche loader.swf et le reste)
fn url_dossier(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if s.starts_with('/') {
        format!("file://{}/", encoder(s, b"/"))
    } else {
        format!("file:///{}/", encoder(s, b"/:"))
    }
}

pub fn port_ouvert(hote: &str, port: u16) -> bool {
    match (hote, port).to_socket_addrs() {
        Ok(adresses) => adresses.into_iter().any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)).is_ok()),
        Err(_) => false,
    }
}

// Efface les fichiers ANKXTRASO* (cache des textes) dans les sauvegardes ; renvoie le nombre effacé
fn effacer_cache_textes(dossier: &Path) -> usize {
    let mut n = 0;
    if let Ok(entrees) = fs::read_dir(dossier) {
        for x in entrees.flatten() {
            let p = x.path();
            if p.is_dir() {
                n += effacer_cache_textes(&p);
            } else if x.file_name().to_string_lossy().starts_with("ANKXTRASO") && fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
    }
    n
}
