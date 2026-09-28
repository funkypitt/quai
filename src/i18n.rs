//! The dock's few strings, in the six languages of the Reader's apps.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Pin,
    Unpin,
    NewWindow,
    Close,
    CloseAll,
    GroupWindows,
    LightAll,
    MinimizeOnClick,
    Buttons,
    OpenSettings,
    Applications,
    Workspaces,
    Untitled,
    Dock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
    De,
    Es,
    Pt,
    Ru,
}

pub fn lang_from(locale: &str) -> Lang {
    match locale.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("fr") => Lang::Fr,
        Some("de") => Lang::De,
        Some("es") => Lang::Es,
        Some("pt") => Lang::Pt,
        Some("ru") => Lang::Ru,
        _ => Lang::En,
    }
}

fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .map_or(Lang::En, |v| lang_from(&v))
    })
}

pub fn tr(msg: Msg) -> &'static str {
    text(msg, lang())
}

/// "Close 3 windows", with the number in place.
pub fn close_all(n: usize) -> String {
    tr(Msg::CloseAll).replace("{n}", &n.to_string())
}

pub fn text(msg: Msg, lang: Lang) -> &'static str {
    use Lang::*;
    use Msg::*;
    match (msg, lang) {
        (Pin, En) => "Pin to the dock",
        (Pin, Fr) => "Épingler au dock",
        (Pin, De) => "Im Dock behalten",
        (Pin, Es) => "Fijar en el dock",
        (Pin, Pt) => "Fixar na doca",
        (Pin, Ru) => "Закрепить в доке",

        (Unpin, En) => "Unpin from the dock",
        (Unpin, Fr) => "Détacher du dock",
        (Unpin, De) => "Aus dem Dock entfernen",
        (Unpin, Es) => "Quitar del dock",
        (Unpin, Pt) => "Remover da doca",
        (Unpin, Ru) => "Открепить от дока",

        (NewWindow, En) => "New window",
        (NewWindow, Fr) => "Nouvelle fenêtre",
        (NewWindow, De) => "Neues Fenster",
        (NewWindow, Es) => "Nueva ventana",
        (NewWindow, Pt) => "Nova janela",
        (NewWindow, Ru) => "Новое окно",

        (Close, En) => "Close",
        (Close, Fr) => "Fermer",
        (Close, De) => "Schließen",
        (Close, Es) => "Cerrar",
        (Close, Pt) => "Fechar",
        (Close, Ru) => "Закрыть",

        (CloseAll, En) => "Close the {n} windows",
        (CloseAll, Fr) => "Fermer les {n} fenêtres",
        (CloseAll, De) => "Alle {n} Fenster schließen",
        (CloseAll, Es) => "Cerrar las {n} ventanas",
        (CloseAll, Pt) => "Fechar as {n} janelas",
        (CloseAll, Ru) => "Закрыть все окна ({n})",

        (GroupWindows, En) => "Group windows by application",
        (GroupWindows, Fr) => "Regrouper les fenêtres par application",
        (GroupWindows, De) => "Fenster nach Anwendung gruppieren",
        (GroupWindows, Es) => "Agrupar las ventanas por aplicación",
        (GroupWindows, Pt) => "Agrupar as janelas por aplicação",
        (GroupWindows, Ru) => "Группировать окна по приложениям",

        (LightAll, En) => "Light every tile",
        (LightAll, Fr) => "Éclairer toutes les tuiles",
        (LightAll, De) => "Alle Kacheln beleuchten",
        (LightAll, Es) => "Iluminar todas las casillas",
        (LightAll, Pt) => "Iluminar todos os mosaicos",
        (LightAll, Ru) => "Подсвечивать все плитки",

        (MinimizeOnClick, En) => "A click minimizes the application in use",
        (MinimizeOnClick, Fr) => "Un clic réduit l'application en cours",
        (MinimizeOnClick, De) => "Ein Klick minimiert die aktive Anwendung",
        (MinimizeOnClick, Es) => "Un clic minimiza la aplicación en uso",
        (MinimizeOnClick, Pt) => "Um clique minimiza a aplicação em uso",
        (MinimizeOnClick, Ru) => "Щелчок сворачивает активное приложение",

        (Buttons, En) => "Applications and workspaces buttons",
        (Buttons, Fr) => "Boutons Applications et Espaces de travail",
        (Buttons, De) => "Schaltflächen für Anwendungen und Arbeitsflächen",
        (Buttons, Es) => "Botones de aplicaciones y espacios de trabajo",
        (Buttons, Pt) => "Botões de aplicações e áreas de trabalho",
        (Buttons, Ru) => "Кнопки приложений и рабочих столов",

        (OpenSettings, En) => "Open the settings file",
        (OpenSettings, Fr) => "Ouvrir le fichier de réglages",
        (OpenSettings, De) => "Einstellungsdatei öffnen",
        (OpenSettings, Es) => "Abrir el archivo de ajustes",
        (OpenSettings, Pt) => "Abrir o ficheiro de definições",
        (OpenSettings, Ru) => "Открыть файл настроек",

        (Applications, En) => "Applications",
        (Applications, Fr) => "Applications",
        (Applications, De) => "Anwendungen",
        (Applications, Es) => "Aplicaciones",
        (Applications, Pt) => "Aplicações",
        (Applications, Ru) => "Приложения",

        (Workspaces, En) => "Workspaces",
        (Workspaces, Fr) => "Espaces de travail",
        (Workspaces, De) => "Arbeitsflächen",
        (Workspaces, Es) => "Espacios de trabajo",
        (Workspaces, Pt) => "Áreas de trabalho",
        (Workspaces, Ru) => "Рабочие столы",

        (Untitled, En) => "Untitled window",
        (Untitled, Fr) => "Fenêtre sans titre",
        (Untitled, De) => "Fenster ohne Titel",
        (Untitled, Es) => "Ventana sin título",
        (Untitled, Pt) => "Janela sem título",
        (Untitled, Ru) => "Окно без названия",

        (Dock, En) => "Dock",
        (Dock, Fr) => "Dock",
        (Dock, De) => "Dock",
        (Dock, Es) => "Dock",
        (Dock, Pt) => "Doca",
        (Dock, Ru) => "Док",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales() {
        assert_eq!(lang_from("fr_CH.UTF-8"), Lang::Fr);
        assert_eq!(lang_from("pt_BR"), Lang::Pt);
        assert_eq!(lang_from("C"), Lang::En);
        assert_eq!(lang_from(""), Lang::En);
    }

    #[test]
    fn the_count_has_its_place_in_every_language() {
        for l in [Lang::En, Lang::Fr, Lang::De, Lang::Es, Lang::Pt, Lang::Ru] {
            assert!(text(Msg::CloseAll, l).contains("{n}"), "{l:?}");
        }
    }
}
