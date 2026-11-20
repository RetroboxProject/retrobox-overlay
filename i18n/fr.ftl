# retrobox-overlay: fr
menu-continue = Continuer
menu-save-state = Sauvegarder l'état
menu-load-state = Charger l'état
menu-manual = Manuel d'instructions
menu-info = Fiche du jeu
menu-controls = Commandes
menu-achievements = Succès
menu-disc-tray = Ouvrir/fermer le tiroir
menu-open-tray = Ouvrir le tiroir
menu-close-tray = Fermer le tiroir
menu-next-disc = Disque suivant
menu-prev-disc = Disque précédent
menu-reset = Réinitialiser
menu-emulator-menu = Menu de l'émulateur
menu-screenshot = Capture d'écran
menu-quit = Quitter le jeu
menu-save-quit = Sauvegarder et quitter
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Alimentation
menu-standby = Mode veille
menu-reboot = Redémarrer
menu-poweroff = Éteindre
menu-performance = Profil de performances
menu-brightness = Luminosité
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Emplacement { $number }
slot-empty = Vide
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A %-d %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disque { $current } sur { $count }
disc-tray-open = tiroir ouvert

# help prompts (EmulationStation's words)
help-choose = CHOISIR
help-select = SÉLECTIONNER
help-hold-unsaved = MAINTENIR : QUITTER SANS SAUVEGARDER
help-back = RETOUR
help-close = FERMER
help-adjust = RÉGLER
help-mute = MUET
help-zoom = ZOOM
help-zoom-in = ZOOM AVANT
help-zoom-out = ZOOM ARRIÈRE
help-move = DÉPLACER
help-page = PAGE

menu-audio = Audio
audio-output = Périphérique de sortie
audio-profile = Profil
audio-no-outputs = Aucune sortie audio trouvée
audio-loading = Chargement…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Accents

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DÉFILER
help-cancel = ANNULER
task-yes = Oui
task-no = Non
task-ok = OK
task-ask = { $label } n'est pas installé. L'installer maintenant ?
task-installing = Installation de { $label }…
task-failed = Impossible d'installer { $label }
task-done = { $label } est installé
task-reinstall-ask = Réinstaller { $label } ? Il sera mis à jour.
task-reinstalling = Réinstallation de { $label }…
task-reinstall-failed = Impossible de mettre à jour { $label }
task-reinstalled = { $label } est à jour
task-downloading = Téléchargement de { $label }…
task-download-failed = Impossible de télécharger { $label }
task-exit-status = L'installateur s'est terminé avec le statut { $status }.
task-still-missing = L'installateur a terminé, mais { $path } est absent.
task-log = Journal : { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Développeur
info-publisher = Éditeur
info-released = Sortie
info-genre = Genre
info-players = Joueurs
info-rating = Note
info-play-count = Parties jouées
info-play-time = Temps de jeu
break-title = Une pause ?
break-text = Vous jouez depuis { $time }. Reposez vos yeux et étirez-vous un peu.
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Pointeur de la souris
controls-wheel = Molette de la souris
controls-click-left = Clic gauche
controls-click-right = Clic droit
controls-click-middle = Clic du milieu
controls-command = Commande
controls-hold = maintenir

# the achievements page (RetroAchievements)
achievements-loading = Chargement des succès…
achievements-failed = Impossible de joindre RetroAchievements
achievements-none = Ce jeu n'a pas de succès
achievements-summary = { $earned } sur { $total } · { $points } sur { $max } points
