# retrobox-overlay: ru
menu-continue = Продолжить
menu-save-state = Сохранить состояние
menu-load-state = Загрузить состояние
menu-manual = Инструкция
menu-disc-tray = Открыть/закрыть лоток
menu-open-tray = Открыть лоток
menu-close-tray = Закрыть лоток
menu-next-disc = Следующий диск
menu-prev-disc = Предыдущий диск
menu-reset = Перезапуск
menu-emulator-menu = Меню эмулятора
menu-screenshot = Снимок экрана
menu-quit = Выйти из игры
# the system menu (sliders and sleep, in every context; sleep outside games)
menu-standby = Спящий режим
menu-brightness = Яркость
menu-volume = Громкость
# save state slots; the date is a strftime format (chrono)
slot-label = Слот { $number }
slot-empty = Пусто
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %-d %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Диск { $current } из { $count }
disc-tray-open = лоток открыт

# help prompts (EmulationStation's words)
help-choose = ВЫБРАТЬ
help-select = ВЫБОР
help-back = НАЗАД
help-close = ЗАКРЫТЬ
help-adjust = НАСТРОИТЬ
help-mute = БЕЗ ЗВУКА
help-zoom = МАСШТАБ
help-zoom-in = УВЕЛИЧИТЬ
help-zoom-out = УМЕНЬШИТЬ
help-move = ДВИГАТЬ
help-page = СТРАНИЦА

menu-audio = Звук
audio-output = Устройство вывода
audio-profile = Профиль
audio-no-outputs = Аудиовыходы не найдены
audio-loading = Загрузка…
help-audio = ЗВУК
# the on-screen keyboard's keys
osk-accents = Акценты

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = ПРОКРУТКА
help-cancel = ОТМЕНА
task-yes = Да
task-no = Нет
task-ok = ОК
task-ask = { $label } не установлен. Установить сейчас?
task-installing = Установка: { $label }…
task-failed = Не удалось установить: { $label }
task-done = { $label } установлен
task-reinstall-ask = Установить { $label } снова? Он будет обновлён.
task-reinstalling = Повторная установка: { $label }…
task-reinstall-failed = Не удалось обновить: { $label }
task-reinstalled = { $label } обновлён
task-downloading = Загрузка: { $label }…
task-download-failed = Не удалось загрузить: { $label }
task-exit-status = Установщик завершился со статусом { $status }.
task-still-missing = Установщик завершил работу, но { $path } отсутствует.
task-log = Журнал: { $path }
