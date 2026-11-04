# Native review needed: the first translations (L10N-2, 2026-11-04)

The first pt-BR, ja, fr, ru, it and de tables of the shell unit (`crates/shell/locale/shell/<lang>.tsv`, 380 keys: the installer, Setup, Settings > Language & Region and the shell's sentence table `copy.rs`) were written in one pass by one worker, at the lead's request, not by one dedicated translator per language as this folder's README asks. Every line follows its RULES page; none has been read by a native speaker yet. Until a reviewer signs a language off here, treat its table as a draft that ships. Below: the choices a reviewer should look at first.

## Every language

- Button names quoted in sentences ("click Try Again") use that language's button text and its quotation marks; check that each quoted name matches the button it points to.
- "Secure Boot": kept in English in de (the firmware menus say it); translated in the others (*Inicialização Segura*, セキュアブート, *démarrage sécurisé*, *безопасная загрузка*, *avvio protetto*).
- The firmware's own menu words (Enroll MOK, Continue, Yes, Reboot) stay in English: the blue key screen shows them in English.
- Theme names (Wonderland, Ocean, Slate) and keyboard layout names ("Brazilian (ABNT2)", "Japanese") are still English everywhere (shared with Settings; not in the catalog yet).

## pt-BR

- *Boas-vindas* for "Welcome" (neutral, RULES-pt-BR), *pendrive* for "USB stick", *Ajustes do Sistema* for "System Settings", *Agora não* for "Not Now".
- "Erase Disk" is *Apagar disco*; "Manual" stays *Manual*.

## ja

- Polite です/ます in sentences, nouns on buttons; 「」 around quoted button names.
- 「再試行」 for Try Again, 「後で」 for Not Now, 「続ける」 for Continue; check they match the tone of the rest of the system.
- The goldens of the ja pages (`setup-language-ja-1280`, `setup-account-ja-1280`) are baked in the golden image with `noto-fonts-cjk` added (`crates/shell/tests/golden-env/Dockerfile`); the forge must build that image to reproduce them.

## fr

- *vous* everywhere, infinitive on buttons; narrow no-break space inside « » and before ; ? !, no-break space before :.
- *Réglages Système* for "System Settings", *Effacer le disque* for "Erase Disk", *Plus tard* for "Not Now".

## ru

- *вы* lower case, «ёлочки», ё written; "Account name" is *Имя учётной записи* (it is wider than the label column of the account form: see the golden `setup-account-ru-1280`).
- Plural of "About {n} minutes remaining": *Осталось около {n} минуты / минут* (genitive after *около*), one/few/many/other.
- *Системные настройки* for "System Settings", *Стереть диск* for "Erase Disk".

## it

- *tu*; "Welcome" is *Ti diamo il benvenuto* (gender-neutral, longer than the usual *Benvenuto*).
- "Erase Disk" is *Inizializza disco* (the usual Italian term for wiping a disk) and the edit page's "Erase" switch is *Inizializza*; the Method row "Erases one partition, keeps the rest" says *Cancella una partizione*: pick one verb.
- *Impostazioni di Sistema* for "System Settings".

## de

- *du*; „…“ quotes; no space before … (RULES-de).
- "Wi-Fi" kept as written (RULES), although German speakers say *WLAN*; a reviewer may want *WLAN*.
- "Themes" is *Themen*; "Disk" is *Festplatte* throughout (also for SSDs); *Datenträger* would be more exact and more formal.
