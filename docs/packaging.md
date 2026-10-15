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

Проверенная сборка 0.1.0 от 15 октября 2026 года:

| Что измеряется | Размер |
|---|---:|
| DMG для скачивания | 35 002 509 байт — 35.0 MB / 33.4 MiB |
| Установленный `.app`, содержимое файлов | 73 789 701 байт — 73.8 MB / 70.4 MiB |
| Выделенное место `.app` на текущей файловой системе | 72 144 KiB — 70.5 MiB |

Архитектура — **arm64 / Apple Silicon**, минимум этой конкретной сборки —
**macOS 15.0**, установленный по наибольшему Mach-O минимуму её библиотек.
Для Intel и более старых ОС нужны отдельно собранные совместимые библиотеки
и проверки на целевых системах. Эта сборка подписана ad-hoc для локального
тестирования, без Developer ID и нотарификации Apple.

Проверены целостность DMG, подписи, запуск из смонтированного образа и загрузка
всех 14 dylib из пакета. Ссылок на Homebrew в загрузке пакета нет. Проверка
запуска выполнена на текущем Mac; нижняя версия ОС отдельно не проверялась.

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

## Место на диске и RAM

Размер установленного приложения не включает пользовательские исходники,
экспорты, проекты и растущие пользовательские кеши. Упаковка в DMG или EXE
не устанавливает предел оперативной памяти. Наблюдаемые расходы RAM,
условия замеров и CPU-only проверка с лимитом 1.5 GiB приведены в
[performance.md](performance.md).
