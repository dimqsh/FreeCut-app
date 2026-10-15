# Локальные пакеты FreeCut

## macOS

На Mac с Python 3.11+, Rust, FFmpeg и Xcode command-line tools:

```sh
python3 scripts/package_macos.py
```

Скрипт собирает release, копирует все несистемные dylib внутрь `FreeCut.app`,
переписывает пути загрузчика, проверяет зависимости и подписи, затем создаёт
сжатый DMG. Шрифты, LUT и пресеты уже встроены в исполняемый файл. Homebrew
на машине, где запускается пакет, не требуется. В DMG есть ссылка Applications;
приложение устанавливается перетаскиванием.

Результаты находятся в `target/dist/`: `.app`, `.dmg` и JSON с размерами,
библиотеками и SHA-256. Существующие результаты не перезаписываются;
для следующей сборки можно передать `--output target/dist-next`.

[Опубликованный macOS Preview 0.1.0](https://github.com/dimqsh/FreeCut-app/releases/tag/v0.1.0-macos-preview)
от 15 октября 2026 года, исходный commit `79d120197dffeae1e465152e4e333b66cfef3175`:

| Что измеряется | Размер |
|---|---:|
| DMG для скачивания | 35 935 283 байта — 35.9 MB / 34.3 MiB |
| Установленный `.app`, содержимое файлов | 75 650 083 байта — 75.7 MB / 72.1 MiB |

Архитектура — **arm64 / Apple Silicon**, минимум этой конкретной сборки —
**macOS 15.0**, установленный по наибольшему Mach-O минимуму её библиотек.
Для Intel и более старых ОС нужны отдельно собранные совместимые библиотеки
и проверки на целевых системах. Эта сборка подписана ad-hoc для локального
тестирования, без Developer ID и нотарификации Apple.

Пакет [собран на macOS 15 / arm64](https://github.com/dimqsh/FreeCut-app/actions/runs/38740223037).
Проверены целостность DMG, подписи, запуск из смонтированного образа и загрузка
всех 16 dylib из пакета, без библиотек Homebrew. Из этой копии создан новый
проект с текстом и экспортирован H.264 MP4: 1920×1080, 30 fps, 90 кадров /
3 секунды; декодирование и наличие титра проверены. Запуск проверен на
Apple M3 Pro / macOS 27.0.1; нижняя версия ОС отдельно не проверялась.
Состав пакета, SHA-256 и результаты проверки приложены к релизу как JSON.

## Windows

Сборка запускается workflow `.github/workflows/windows-package.yml` на настоящем
Windows x64 runner. На разработческой Windows с Rust, Visual Studio C++ tools,
LLVM и Python 3.11+ те же шаги можно выполнить вручную:

```powershell
./scripts/setup_windows_ffmpeg.ps1
$env:RUSTFLAGS = '-C target-cpu=x86-64 -C target-feature=+crt-static'
cargo build -p freecut-app --release --target x86_64-pc-windows-msvc --locked
python scripts/package_windows.py --ffmpeg $env:FFMPEG_DIR
./scripts/check_windows_cpu.ps1
```

В `target/windows-dist/` создаются portable ZIP, JSON со списком фактически
нужных DLL и SHA-256. Inno Setup собирает установщик по
`scripts/windows-installer.iss`; установка идёт в каталог текущего пользователя
без обязательных прав администратора. Кодеки поставляются вместе с приложением;
Python, Rust, FFmpeg CLI и инструменты разработки для запуска не нужны.

`FreeCut-CPU.cmd` в portable-пакете и ярлык **FreeCut (CPU)** установщика
передают `--software --low-memory`. На Windows это только DX12/WARP, компилятор
FXC из ОС и малые бюджеты ресурсов. Аппаратный GPU этим путём не выбирается.
Режим не меняет разрешение экспорта, операции монтажа или оформление UI.

### Совместимость

Проверенная Windows-сборка от 15 октября 2026 года:

| Файл / установка | Размер |
|---|---:|
| Установщик EXE | 57 563 230 байт — 57.6 MB |
| Portable ZIP | 79 390 378 байт — 79.4 MB |
| Установленное приложение с кодеками | 207 934 723 байта — 207.9 MB |
| Сам `freecut.exe`, без DLL | 49 336 832 байта — 49.3 MB |

[Windows-сборка и тесты](https://github.com/dimqsh/FreeCut-app/actions/runs/38725250476)
прошли: 121 unit-тест рендера, 13 e2e-тестов, включая экспорт MP4 с текстом
и звуком. Найденная Windows-ошибка финализации экспорта исправлена: для
`FlushFileBuffers` готовый файл открывается с правом записи, без усечения.

Portable и установленная копия также прошли
[отдельную проверку на чистом Windows-runner](https://github.com/dimqsh/FreeCut-app/actions/runs/38730326080):
в PATH приложения оставлены только Windows/System32 и Windows, переменная
FFMPEG_DIR удалена. Обе копии создали окно на **Microsoft Basic Render Driver
(WARP)** и нормально завершились с действующим лимитом 1536 MiB committed
memory процесса. Привязка CPU — два ядра. Наблюдения для Home через 15 секунд:

| Метрика | Portable | После установки |
|---|---:|---:|
| Working set | 63.0 MiB | 62.2 MiB |
| Peak working set | 63.6 MiB | 62.9 MiB |
| Peak committed memory | 288.2 MiB | 287.1 MiB |

Это память запуска и Home; расходы монтажа и экспорта зависят от проекта.
Working set, commit и общий RAM компьютера — разные показатели. В тесте
была Windows Server 2022, build 20348; физический старый ПК с 2 ГБ общей RAM
не проверялся. Ограничение Job Object не распространялось на всю ОС.
Состав пакета, compiler/flags, контрольные суммы и точные значения сохранены
в JSON рядом с файлами в `target/dist/windows/` и в `performance-results.json`.

- Цель этой сборки: **Windows 10 22H2 x64 и Windows 11 x64**. Реальная проверка
  запуска выполняется на Windows Server runner, поэтому это ещё не испытание
  физического старого ПК с 2 ГБ общей RAM.
- `target-cpu=x86-64`, без обязательных AVX/AVX2 в Rust-коде; CRT Rust
  статический. Зависимости FFmpeg закреплены URL и SHA-256 в
  `scripts/windows-ffmpeg.json`. Упаковщик читает обычные и delay-load PE imports,
  включает транзитивные DLL и проверяет отсутствие необеспеченного VC++ runtime.
- Проверка окна использует Job Object с пределом 1536 MiB **committed memory
  процесса**, проверяет применение лимита и записывает peak commit и working set.
  Это не лимит всей RAM компьютера и не эмуляция старого процессора. Отдельно
  выполняются компактные unit-тесты рендера и e2e монтажа/экспорта.
- **Windows 7/8, 32-bit Windows и все старые ПК не заявлены совместимыми.**
  Обычный [Rust MSVC target требует Windows 10+](https://doc.rust-lang.org/stable/rustc/platform-support/windows-msvc.html).
  Для Windows 7 есть [отдельный Tier 3 target без готовой std](https://doc.rust-lang.org/rustc/platform-support/win7-windows-msvc.html),
  однако он сам по себе не портирует UI и остальные зависимости.
  Используемый DX12/WARP требует Windows 10; Windows 7 имеет другой,
  DX11-вариант WARP. См. [Microsoft WARP Guide](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/directx-warp).
- Минимум дистрибутива FFmpeg — Windows 10 22H2 по
  [описанию поставщика](https://github.com/BtbN/FFmpeg-Builds). Для более старых
  ОС нужны отдельные совместимые зависимости, backend интерфейса и реальные
  испытания. Один универсальный EXE для всех поколений Windows не подготовлен.

### GitHub Releases

Установщик `.exe`, portable ZIP, контрольные суммы и сведения о сборке подходят
для assets GitHub Release. Release должен ссылаться на commit с теми исходниками,
из которых собран пакет. Сначала можно создать черновик с файлами, затем
опубликовать его; [документация GitHub](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository).

Для этого предварительного выпуска тег `v0.1.0-windows-preview` указывает на
проверенный commit `fb1881c2aa180fc8746f9629d698c1a9f3693954`. Перед staging
скрипт `scripts/stage_windows_release.py` проверяет реальный commit тега,
соответствие исходникам в package-info и успешные Windows-проверки. Скрипт
создаёт только draft, без публичной публикации. При существующем теге он
не передаёт `target_commitish` API GitHub: источником выпуска служит тег.

## Место на диске и RAM

Размер установленного приложения не включает пользовательские исходники,
экспорты, проекты и растущие пользовательские кеши. Упаковка в DMG или EXE
не устанавливает предел оперативной памяти. Наблюдаемые расходы RAM,
условия замеров и CPU-only проверка с лимитом 1.5 GiB приведены в
[performance.md](performance.md).
