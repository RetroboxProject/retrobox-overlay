# retrobox-overlay: pl
menu-continue = Kontynuuj
menu-save-state = Zapisz stan
menu-load-state = Wczytaj stan
menu-manual = Instrukcja obsługi
menu-info = Karta gry
menu-controls = Sterowanie
menu-achievements = Osiągnięcia
menu-disc-tray = Otwórz/zamknij tackę
menu-open-tray = Otwórz tackę
menu-close-tray = Zamknij tackę
menu-next-disc = Następna płyta
menu-prev-disc = Poprzednia płyta
menu-reset = Uruchom ponownie
menu-emulator-menu = Menu emulatora
menu-screenshot = Zrzut ekranu
menu-quit = Wyjdź z gry
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Zasilanie
menu-standby = Tryb uśpienia
menu-reboot = Uruchom ponownie
menu-poweroff = Wyłącz
menu-brightness = Jasność
menu-volume = Głośność
# save state slots; the date is a strftime format (chrono)
slot-label = Slot { $number }
slot-empty = Pusty
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %-d %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Płyta { $current } z { $count }
disc-tray-open = tacka otwarta

# help prompts (EmulationStation's words)
help-choose = PRZEJDŹ
help-select = WYBIERZ
help-back = WSTECZ
help-close = ZAMKNIJ
help-adjust = DOSTOSUJ
help-mute = WYCISZ
help-zoom = ZOOM
help-zoom-in = POWIĘKSZENIE
help-zoom-out = POMNIEJSZENIE
help-move = PRZESUŃ
help-page = STRONA

menu-audio = Dźwięk
audio-output = Urządzenie wyjściowe
audio-profile = Profil
audio-no-outputs = Nie znaleziono wyjść audio
audio-loading = Ładowanie…
help-audio = DŹWIĘK
# the on-screen keyboard's keys
osk-accents = Akcenty

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = PRZEWIŃ
help-cancel = ANULUJ
task-yes = Tak
task-no = Nie
task-ok = OK
task-ask = { $label } nie jest zainstalowany. Zainstalować teraz?
task-installing = Instalowanie: { $label }…
task-failed = Nie udało się zainstalować: { $label }
task-done = { $label } jest zainstalowany
task-reinstall-ask = Zainstalować { $label } ponownie? Zostanie zaktualizowany.
task-reinstalling = Ponowne instalowanie: { $label }…
task-reinstall-failed = Nie udało się zaktualizować: { $label }
task-reinstalled = { $label } jest aktualny
task-downloading = Pobieranie: { $label }…
task-download-failed = Nie udało się pobrać: { $label }
task-exit-status = Instalator zakończył działanie ze statusem { $status }.
task-still-missing = Instalator zakończył działanie, ale brak { $path }.
task-log = Dziennik: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Producent
info-publisher = Wydawca
info-released = Premiera
info-genre = Gatunek
info-players = Gracze
info-rating = Ocena
info-play-count = Liczba uruchomień
info-play-time = Czas gry
play-time = { $session } · { $total } łącznie
duration-minutes = { $minutes } min
duration-hours = { $hours } godz.
duration-hours-minutes = { $hours } godz. { $minutes } min
controls-pointer = Kursor myszy
controls-wheel = Kółko myszy
controls-click-left = Lewy przycisk
controls-click-right = Prawy przycisk
controls-click-middle = Środkowy przycisk
controls-command = Polecenie
controls-hold = przytrzymaj

# the achievements page (RetroAchievements)
achievements-loading = Wczytywanie osiągnięć…
achievements-failed = Brak połączenia z RetroAchievements
achievements-none = Ta gra nie ma osiągnięć
achievements-summary = { $earned } z { $total } · { $points } z { $max } pkt
