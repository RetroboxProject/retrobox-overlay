# retrobox-overlay: eu
menu-continue = Jarraitu
menu-save-state = Gorde egoera
menu-load-state = Kargatu egoera
menu-manual = Argibide-eskuliburua
menu-info = Jokoaren fitxa
menu-controls = Kontrolak
menu-achievements = Lorpenak
menu-disc-tray = Ireki/itxi erretilua
menu-open-tray = Ireki erretilua
menu-close-tray = Itxi erretilua
menu-next-disc = Hurrengo diskoa
menu-prev-disc = Aurreko diskoa
menu-reset = Berrabiarazi
menu-emulator-menu = Emuladorearen menua
menu-screenshot = Pantaila-argazkia
menu-quit = Irten jokotik
menu-save-quit = Gorde eta irten
menu-quit-unsaved = Irten gorde gabe
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Energia
menu-standby = Lo modua
menu-reboot = Berrabiarazi
menu-poweroff = Itzali
menu-brightness = Distira
menu-volume = Bolumena
# save state slots; the date is a strftime format (chrono)
slot-label = { $number }. zirrikitua
slot-empty = Hutsik
slot-date-format = %Y %b %-d · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %Y.eko %Bren %-da
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = { $count }tik { $current }. diskoa
disc-tray-open = erretilua irekita

# help prompts (EmulationStation's words)
help-choose = AUKERATU
help-select = HAUTATU
help-back = ATZERA
help-close = ITXI
help-adjust = DOITU
help-mute = ISILDU
help-zoom = ZOOMA
help-zoom-in = HANDIAGOTU
help-zoom-out = TXIKIAGOTU
help-move = MUGITU
help-page = ORRIA

menu-audio = Audioa
audio-output = Irteerako gailua
audio-profile = Profila
audio-no-outputs = Ez da audio-irteerarik aurkitu
audio-loading = Kargatzen…
help-audio = AUDIOA
# the on-screen keyboard's keys
osk-accents = Azentuak

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DESPLAZATU
help-cancel = UTZI
task-yes = Bai
task-no = Ez
task-ok = Ados
task-ask = { $label } ez dago instalatuta. Orain instalatu?
task-installing = { $label } instalatzen…
task-failed = Ezin izan da { $label } instalatu
task-done = { $label } instalatuta dago
task-reinstall-ask = { $label } berriro instalatu? Eguneratu egingo da.
task-reinstalling = { $label } berriro instalatzen…
task-reinstall-failed = Ezin izan da { $label } eguneratu
task-reinstalled = { $label } eguneratuta dago
task-downloading = { $label } deskargatzen…
task-download-failed = Ezin izan da { $label } deskargatu
task-exit-status = Instalatzaileak { $status } egoerarekin amaitu du.
task-still-missing = Instalatzailea amaitu da, baina { $path } ez dago.
task-log = Erregistroa: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Garatzailea
info-publisher = Argitaratzailea
info-released = Argitaratzea
info-genre = Generoa
info-players = Jokalariak
info-rating = Balorazioa
info-play-count = Jokatutako aldiak
info-play-time = Jokatutako denbora
play-time = { $session } · { $total } guztira
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Saguaren erakuslea
controls-wheel = Saguaren gurpila
controls-click-left = Ezkerreko klika
controls-click-right = Eskuineko klika
controls-click-middle = Erdiko klika
controls-command = Komandoa
controls-hold = eutsi

# the achievements page (RetroAchievements)
achievements-loading = Lorpenak kargatzen…
achievements-failed = Ezin da RetroAchievements-ekin konektatu
achievements-none = Joko honek ez du lorpenik
achievements-summary = { $earned }/{ $total } · { $points }/{ $max } puntu
