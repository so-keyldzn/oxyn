//! Un serveur HTTP d'essai sur la boucle locale.
//!
//! Assez pour rejouer ce qu'un fournisseur hostile peut faire — rediriger,
//! répondre un corps qui ne finit pas, diffuser des trames —, et rien de
//! plus : chaque connexion reçoit la réponse brute qu'on lui a donnée, et
//! ce qui a été reçu reste consultable.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Ce qu'une connexion a reçu : en-têtes et corps, en texte.
pub(crate) type Received = Arc<Mutex<Vec<String>>>;

/// Un serveur qui répond `reply` à chaque connexion.
///
/// Tant que `hold` n'est pas relâché, la connexion reste ouverte après la
/// réponse : c'est ce qui simule un corps qui ne se termine pas.
pub(crate) struct Server {
    /// `http://127.0.0.1:<port>`, sans barre finale.
    pub(crate) origin: String,
    /// Tout ce qui a été reçu, une entrée par requête.
    pub(crate) received: Received,
    /// Relâché à la destruction du serveur : les connexions retenues se
    /// ferment alors.
    _release: oneshot::Sender<()>,
}

impl Server {
    /// Démarre un serveur qui répond `reply`, puis ferme — ou retient la
    /// connexion si `hold` est vrai.
    pub(crate) async fn start(reply: impl Into<Vec<u8>>, hold: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let origin = format!("http://{}", listener.local_addr().expect("a bound address"));
        let received: Received = Arc::new(Mutex::new(Vec::new()));
        let (release, released) = oneshot::channel::<()>();
        let released = futures::future::FutureExt::shared(released);
        let reply = reply.into();
        let recorded = Arc::clone(&received);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let reply = reply.clone();
                let recorded = Arc::clone(&recorded);
                let released = released.clone();
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    recorded.lock().expect("test lock").push(request);
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.flush().await;
                    if hold {
                        let _ = released.await;
                    }
                });
            }
        });
        Self {
            origin,
            received,
            _release: release,
        }
    }

    /// Toutes les requêtes reçues, en un seul texte.
    pub(crate) fn seen(&self) -> String {
        self.received.lock().expect("test lock").join("\n")
    }

    /// Nombre de requêtes reçues.
    pub(crate) fn hits(&self) -> usize {
        self.received.lock().expect("test lock").len()
    }
}

/// Lit les en-têtes, puis le corps annoncé par `content-length`.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut lu = Vec::new();
    let mut tampon = [0_u8; 4096];
    while let Ok(n) = socket.read(&mut tampon).await {
        if n == 0 {
            break;
        }
        lu.extend_from_slice(tampon.get(..n).unwrap_or_default());
        let texte = String::from_utf8_lossy(&lu).into_owned();
        if let Some(fin) = texte.find("\r\n\r\n") {
            let attendu = texte
                .lines()
                .find_map(|ligne| {
                    let (nom, valeur) = ligne.split_once(':')?;
                    nom.eq_ignore_ascii_case("content-length")
                        .then(|| valeur.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if lu.len() >= fin + 4 + attendu {
                break;
            }
        }
    }
    String::from_utf8_lossy(&lu).into_owned()
}

/// Une réponse de redirection vers `location`.
pub(crate) fn redirect(status: u16, location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Redirect\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    )
    .into_bytes()
}
