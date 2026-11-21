//! German for the profiles lane's tools outside the Profiles page: the one-skin-per-hero
//! question (`one_skin.rs`), Pack all enabled (`installed/pack_all.rs`) and cursor packs from
//! your own images (`locker/cursor_create.rs`). Chained into `i18n`'s maps.

pub const DE: &[(&str, &str)] = &[
    // One skin per hero
    ("{} now has two skins on", "{} hat jetzt zwei Skins an"),
    ("{} is on as well as {}. The lower slot wins where they replace the same files.", "{} ist an, zusammen mit {}. Wo beide dieselben Dateien ersetzen, gewinnt der niedrigere Slot."),
    ("Keep only the new one", "Nur den neuen behalten"),
    ("Only {} is on for {}", "Nur {} ist für {} an"),
    ("The other skins were not turned off", "Die anderen Skins wurden nicht ausgeschaltet"),
    ("No Deadlock install was found", "Keine Deadlock-Installation gefunden"),
    ("Nothing was renamed.", "Nichts wurde umbenannt."),
    // Pack all enabled into N
    ("Pack all enabled", "Alle aktiven packen"),
    ("into 2 to 8 VPKs", "in 2 bis 8 VPKs"),
    ("Pack all enabled mods", "Alle aktiven Mods packen"),
    ("Packs", "Pakete"),
    ("Pack {}", "Paket {}"),
    ("{} mods, slots {} to {}", "{} Mods, Slots {} bis {}"),
    ("{} stays as it is.", "{} bleibt, wie er ist."),
    ("Every enabled mod goes into a few shared VPKs, neighbouring slots together, so the load order stays the same. The mods stay on disk switched off, and Unmerge in the merge list takes a pack apart.", "Jeder aktive Mod kommt in einige gemeinsame VPKs, benachbarte Slots zusammen, damit die Ladereihenfolge gleich bleibt. Die Mods bleiben ausgeschaltet auf der Festplatte, und Trennen in der Merge-Liste nimmt ein Paket wieder auseinander."),
    ("Packing", "Wird gepackt"),
    ("Pack {} mods", "{} Mods packen"),
    ("Packed {} mods into {}.", "{} Mods in {} gepackt."),
    // Cursor packs from your own images
    ("Create from images", "Aus Bildern erstellen"),
    ("Create a cursor pack", "Cursor-Paket erstellen"),
    ("Pick an image for this cursor", "Ein Bild für diesen Cursor wählen"),
    ("Pointer", "Zeiger"),
    ("Shop", "Shop"),
    ("Commend", "Loben"),
    ("{} by {}, clicks at {}, {}", "{} mal {}, klickt bei {}, {}"),
    ("Move the click point {}", "Klickpunkt nach {} verschieben"),
    ("left", "links"),
    ("up", "oben"),
    ("down", "unten"),
    ("right", "rechts"),
    ("Deadlock's own stays", "Der von Deadlock bleibt"),
    ("Pick image", "Bild wählen"),
    ("Name for the pack", "Name für das Paket"),
    ("Like My cursors", "Etwa Meine Cursor"),
    ("Pick an image for any cursor you want to change. Sigil guesses where it clicks from the art; move the dot if it guessed wrong. Cursors you leave alone stay Deadlock's own.", "Wähle ein Bild für jeden Cursor, den du ändern willst. Sigil schätzt anhand des Bildes, wo er klickt; verschiebe den Punkt, wenn die Schätzung danebenliegt. Cursor, die du nicht änderst, bleiben die von Deadlock."),
    ("Created {}. Press its card to use it.", "{} erstellt. Drücke seine Karte, um es zu nutzen."),
];
