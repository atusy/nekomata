#!/usr/bin/env bash
# Run in a private GUI session for a native screenshot.
printf '\033[2J\033[H\033[?25l'
printf '\033[38;2;192;160;223mbold_font\033[0m auto\n'
printf '\033[38;2;192;160;223mbold_italic_font\033[0m auto\n'
printf '\033[38;2;192;160;223mfont_family\033[0m family="\033[38;2;166;190;130mUDEV Gothic NF\033[0m"\n'
printf '\033[38;2;192;160;223mfont_size\033[0m \033[38;2;242;132;120m15.0\033[0m\n\n'
printf 'abcdef ABCDEF 0123456789 @#%%&()\n'
printf 'iiii llll mmmm WWWW / \\ eeee\n'
printf '\033[1mBold\033[0m  \033[3mItalic\033[0m  \033[2mFaint\033[0m  \033[7mReverse\033[0m\n'
printf '\033[4mUnderline\033[0m  \033[4:3mCurly\033[0m  \033[9mStrike\033[0m\n'
printf '日本語 / é / ① ★ ⚙ / 🦀 🍎\n\n'
printf '\033[48;2;230;230;230m\033[38;2;36;34;53mLight background: abcdef 0123456789\033[0m\n'
printf '\n'
