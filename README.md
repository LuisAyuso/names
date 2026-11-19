# names

A baby name picker for two phones. Each of you swipes through names on your
own phone, like a dating app: right for yes, left for no. The phones find each
other on the same Wi-Fi and share what you have swiped, and the names you both
like are revealed together.

## Install

Download `names.apk` from the
[latest release](https://github.com/LuisAyuso/names/releases/latest) on the
phone and open it. Android asks to allow installing apps from your browser;
allow it. On Samsung phones, turn off Auto Blocker (*Automatische Sperre*)
while installing.

## How it works

- **Swipe**: one name at a time. Every right swipe adds a point to the name,
  every left swipe takes one away. Names come back now and then: liked ones
  every 10, 20, 30… cards, discarded ones every 20, 40, 60….
- **Agreed**: the names you both scored above zero, highest combined score
  first, each tinted pink and blue by how much each of you likes it. The list
  stays hidden until both phones are on the same Wi-Fi and one of you taps
  **Reveal!**.
- **Menu (☰)**:
  - **Names**: every name with your score; add your own, or tap one to reset
    its score.
  - **Language**: English or Deutsch.
  - **Reset everything**: starts over on both phones.

The app comes with over 300 Austrian and Spanish girls' names, from
traditional (Resi, Notburga, Covadonga) to current (Mila, Aitana). Nothing leaves your Wi-Fi: there is no account and no server.

## More

- [Architecture](docs/architecture.md): how the app and the sync work.
- [Development](docs/development.md): building, testing and releasing.

Built with [dfui](https://github.com/LuisAyuso/dfui).
