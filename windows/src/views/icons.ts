// SVG paths standing in for the SF Symbols used by the macOS island.
// Drawn on a 24×24 grid so they read at the same optical size.

export const ICONS = {
  // house.fill
  house: "M12 3.2 2.8 10.6V21h6.6v-5.4h5.2V21h6.6V10.6L12 3.2z",
  // bubble.left.fill
  bubble: "M12 3.6c-5 0-9 3.3-9 7.4 0 2.3 1.3 4.4 3.3 5.7-.2 1.2-.8 2.4-1.7 3.4 1.9-.2 3.6-.9 4.9-1.9 .8.2 1.6.3 2.5.3 5 0 9-3.3 9-7.5s-4-7.4-9-7.4z",
  // plus
  plus: "M11 4h2v7h7v2h-7v7h-2v-7H4v-2h7V4z",
  // gearshape
  gear: "M12 8.6a3.4 3.4 0 1 0 0 6.8 3.4 3.4 0 0 0 0-6.8zm0 1.8a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2zM10.9 2h2.2l.35 2.1c.6.17 1.16.4 1.67.71l1.9-1 1.55 1.55-1 1.9c.3.5.54 1.07.7 1.67l2.13.35v2.2l-2.12.35c-.17.6-.4 1.16-.71 1.67l1 1.9-1.55 1.55-1.9-1c-.5.3-1.07.54-1.67.7L13.1 22h-2.2l-.35-2.12c-.6-.17-1.16-.4-1.67-.71l-1.9 1L5.43 18.6l1-1.9c-.3-.5-.54-1.07-.7-1.67L3.6 14.7v-2.2l2.12-.35c.17-.6.4-1.16.71-1.67l-1-1.9 1.55-1.55 1.9 1c.5-.3 1.07-.54 1.67-.7L10.9 2z",
  gearFill: "M10.9 2h2.2l.35 2.1c.6.17 1.16.4 1.67.71l1.9-1 1.55 1.55-1 1.9c.3.5.54 1.07.7 1.67l2.13.35v2.2l-2.12.35c-.17.6-.4 1.16-.71 1.67l1 1.9-1.55 1.55-1.9-1c-.5.3-1.07.54-1.67.7L13.1 22h-2.2l-.35-2.12c-.6-.17-1.16-.4-1.67-.71l-1.9 1L5.43 18.6l1-1.9c-.3-.5-.54-1.07-.7-1.67L3.6 14.7v-2.2l2.12-.35c.17-.6.4-1.16.71-1.67l-1-1.9 1.55-1.55 1.9 1c.5-.3 1.07-.54 1.67-.7L10.9 2zM12 8.2a3.8 3.8 0 1 0 0 7.6 3.8 3.8 0 0 0 0-7.6z",
  // speaker.wave.2
  speakerOn: "M11 4.5 6.5 8.2H3.4v7.6h3.1L11 19.5v-15zm3.2 3a5.3 5.3 0 0 1 0 9 .9.9 0 0 0 .9 1.55 7.1 7.1 0 0 0 0-12.1.9.9 0 0 0-.9 1.55zm2.6-3.1a8.9 8.9 0 0 1 0 15.2.9.9 0 0 0 .92 1.55 10.7 10.7 0 0 0 0-18.3.9.9 0 0 0-.92 1.55z",
  // speaker.slash
  speakerOff: "M11 4.5 6.5 8.2H3.4v7.6h3.1L11 19.5v-15zm3.6 4.1 1.27-1.27 2.33 2.33 2.33-2.33 1.27 1.27L19.47 11l2.33 2.33-1.27 1.27-2.33-2.33-2.33 2.33-1.27-1.27L16.93 11 14.6 8.6z",
  // arrow.up.right
  arrowUpRight: "M8.5 7h8.5v8.5h-2V10.4l-7.1 7.1-1.4-1.4 7.1-7.1H8.5V7z",
  // chevron.right
  chevronRight: "M9 5.5 15.5 12 9 18.5",
  chevronLeft: "M15 5.5 8.5 12 15 18.5",
  // checkmark
  check: "M5 12.5 9.5 17 19 7.5",
  // arrow.up (send)
  arrowUp: "M12 4.5 5.5 11l1.5 1.5 4-4V19.5h2V8.5l4 4L18.5 11 12 4.5z",
  // exclamationmark
  bang: "M11 4h2v10h-2V4zm0 12.2h2v2.2h-2v-2.2z",
  // xmark
  xmark: "M6.4 5 12 10.6 17.6 5 19 6.4 13.4 12 19 17.6 17.6 19 12 13.4 6.4 19 5 17.6 10.6 12 5 6.4 6.4 5z",
  // timer
  timer: "M12 4.2a7.8 7.8 0 1 0 0 15.6 7.8 7.8 0 0 0 0-15.6zm0 1.9a5.9 5.9 0 1 1 0 11.8 5.9 5.9 0 0 1 0-11.8zm-.95 2.3v4.2l3.3 2 .95-1.55-2.4-1.45V8.4h-1.85zM9.2 2h5.6v1.7H9.2V2z",
  // ellipsis
  ellipsis: "M6 10.4a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2zm6 0a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2zm6 0a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2z",
  // star.fill
  star: "M12 3.2l2.6 5.55 5.9.82-4.3 4.3 1.05 6.13L12 17.1l-5.25 2.9L7.8 13.87 3.5 9.57l5.9-.82L12 3.2z",
  // square.stack.fill
  stack: "M5 8h14v11.5H5V8zm1.8-3h10.4v1.6H6.8V5zm1.6-2.6h7.2V4H8.4V2.4z",
  // list.bullet (sessions tab)
  list: "M4.6 5.6a1.4 1.4 0 1 0 0 2.8 1.4 1.4 0 0 0 0-2.8zM8.5 6.1h11v1.8h-11V6.1zm-3.9 4.5a1.4 1.4 0 1 0 0 2.8 1.4 1.4 0 0 0 0-2.8zm3.9.5h11v1.8h-11v-1.8zm-3.9 4.5a1.4 1.4 0 1 0 0 2.8 1.4 1.4 0 0 0 0-2.8zm3.9.5h11v1.8h-11v-1.8z",
  // doc.text
  doc: "M6.5 2.6h7l4 4v14.8h-11V2.6zm6.6 1.6v3.3h3.3l-3.3-3.3zM8.6 11h6.8v1.5H8.6V11zm0 3.4h6.8v1.5H8.6v-1.5z",
  // bell.fill (quiet mode off)
  bell: "M12 2.8a1.5 1.5 0 0 0-1.5 1.5v.6A6 6 0 0 0 6 10.8v4.4L4.2 17.4v1.1h15.6v-1.1L18 15.2v-4.4a6 6 0 0 0-4.5-5.9v-.6A1.5 1.5 0 0 0 12 2.8zm-2.2 16.9a2.2 2.2 0 0 0 4.4 0h-4.4z",
  // bell.slash.fill (quiet mode on): the slash is cut out of the bell
  bellSlash: "M15.1 18.5 4.2 18.5 4.2 17.4 6.0 15.2 6.0 10.8 6.0 10.8 6.1 9.8 6.1 9.5zM10.5 4.3 10.7 3.6 11.2 3.0 12.0 2.8 12.7 3.0 13.3 3.6 13.5 4.3 13.5 5.0 14.4 5.3 15.3 5.8 16.1 6.4 16.7 7.1 17.3 7.9 17.7 8.9 17.9 9.8 18.0 10.8 18.0 14.6 9.0 5.6 9.6 5.3 10.5 5.0zM9.8 19.7 14.2 19.7 14.0 20.5 13.6 21.3 12.8 21.7 12.0 21.9 11.2 21.7 10.4 21.3 10.0 20.5zM3.9 2.5 21.5 20.1 20.1 21.5 2.5 3.9z",
  // scope / target (focus mode): two rings and a dot, the gaps cut out by opposite winding
  target: "M12 3a9 9 0 1 1 0 18 9 9 0 1 1 0-18zm0 1.8a7.2 7.2 0 1 0 0 14.4 7.2 7.2 0 1 0 0-14.4zm0 1.8a5.4 5.4 0 1 1 0 10.8 5.4 5.4 0 1 1 0-10.8zm0 1.8a3.6 3.6 0 1 0 0 7.2 3.6 3.6 0 1 0 0-7.2zm0 1.8a1.8 1.8 0 1 1 0 3.6 1.8 1.8 0 1 1 0-3.6z",
  // chevron.up (collapse)
  chevronUp: "M5.5 15 12 8.5 18.5 15",
  // The player row (night spec §B).
  play: "M8 5.5v13l10.5-6.5L8 5.5z",
  pause: "M7 5.5h3.4v13H7v-13zm6.6 0H17v13h-3.4v-13z",
  skipNext: "M5.5 6v12l8.5-6-8.5-6zm9.5 0h2.5v12H15V6z",
  skipPrev: "M18.5 6v12L10 12l8.5-6zM6.5 6H9v12H6.5V6z",
  // shuffle, repeat, repeat.1 (ideas §35): strokes, drawn with svg(…, { stroke: 2 }).
  shuffle: "M4 7h3l10 10h3M17 14l3 3-3 3M4 17h3l3-3M14 10l3-3h3M17 4l3 3-3 3",
  repeat: "M4 12V9a2 2 0 0 1 2-2h13M16 4l3 3-3 3M20 12v3a2 2 0 0 1-2 2H5M8 20l-3-3 3-3",
  repeatOne: "M4 12V9a2 2 0 0 1 2-2h13M16 4l3 3-3 3M20 12v3a2 2 0 0 1-2 2H5M8 20l-3-3 3-3M11 10.8l1.3-.8v4.2",
  // chart.bar.fill (the Stats tab, night spec §C)
  bars: "M4 13h4v7H4v-7zm6-9h4v16h-4V4zm6 5h4v11h-4V9z",
  // terminal (the Agents tab): a prompt in a window
  terminal: "M3.5 4.5h17v15h-17v-15zm1.8 1.8v11.4h13.4V6.3H5.3zm1.9 2.2 1.1-1.1 3.3 3.3-3.3 3.3-1.1-1.1 2.2-2.2-2.2-2.2zm5.1 6.4h4.6v1.6h-4.6v-1.6z",
  // stop (a filled square: the chat's stop button)
  stop: "M7 7h10v10H7V7z",
  // square.grid.2x2.fill (the Apps tab)
  grid: "M4 4h7v7H4V4zm9 0h7v7h-7V4zM4 13h7v7H4v-7zm9 0h7v7h-7v-7z",
  // doc.on.doc (a code block's Copy)
  copy: "M8 3h9a2 2 0 0 1 2 2v11h-2V5H8V3zM5 7h9a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2zm0 2v10h9V9H5z",
  // folder.fill (the Agents tab's project chip)
  folder: "M3 6.5A1.5 1.5 0 0 1 4.5 5h4.2l2 2.2h8.8A1.5 1.5 0 0 1 21 8.7v9.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-12z",
  // chevron.down (a chip's menu), drawn as a stroke
  chevronDown: "M5.5 9 12 15.5 18.5 9",
  // lock (an agent that cannot be changed any more), drawn as a stroke
  lock: "M6.5 10.5h11a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1zM8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5",
  // pencil.fill (a mode that may edit files)
  pencil: "M4 17.2V20h2.8L18.6 8.2l-2.8-2.8L4 17.2zM20.7 6.1a1 1 0 0 0 0-1.4l-1.4-1.4a1 1 0 0 0-1.4 0l-1.2 1.2 2.8 2.8 1.2-1.2z",
} as const;
