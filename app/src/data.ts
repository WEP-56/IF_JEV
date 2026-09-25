import type { Story, Settings, Character, World, MapNode, Message } from './types';

export const uid = () => Math.random().toString(36).slice(2, 10);

export const now = () => {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
};

export const JEV_STEPS = ['解析事件语义', '匹配世界规则', '推演状态迁移', '一致性校验', '提交状态快照'];

export const EVENT_TAGS = ['天气', '时间跳跃', '人物', '势力', '灾变', '发现', '自定义'];

/* ------------------------------------------------------------------ */
/* 故事一：雨城纪事（完整示例）                                           */
/* ------------------------------------------------------------------ */

const rainChars: Character[] = [
  {
    id: 'c1',
    name: '沈知遥',
    title: '水文站见习观测员',
    initials: '沈',
    gradient: 'from-sky-400 to-indigo-600',
    tags: ['冷静', '执拗', '数据派'],
    bio: '二十三岁，毕业后被分配到临江水文站。习惯用数字描述世界，却在第一场暴雨中第一次感到数字的无力。',
    stats: [
      { label: '体力', value: 72, color: 'bg-emerald-500' },
      { label: '理智', value: 81, color: 'bg-sky-500' },
      { label: '信任·老周', value: 64, color: 'bg-amber-500' },
    ],
    state: [
      { k: '当前目标', v: '修复 3 号水位计' },
      { k: '携带物品', v: '防水记录本、手电、半包压缩饼干' },
      { k: '秘密', v: '已发现上游水库数据被人为篡改' },
    ],
    locked: true,
    portrait: true,
    location: '临江水文站',
  },
  {
    id: 'c2',
    name: '周望川',
    title: '老站长 / 退伍工程兵',
    initials: '周',
    gradient: 'from-amber-400 to-orange-700',
    tags: ['沉默', '可靠', '旧伤'],
    bio: '在临江守了二十七年的水文站。九八年抗洪时左腿落下旧伤，一到阴雨天就疼。他知道这座城市每一条排水渠的走向。',
    stats: [
      { label: '体力', value: 48, color: 'bg-emerald-500' },
      { label: '理智', value: 90, color: 'bg-sky-500' },
      { label: '腿伤', value: 66, color: 'bg-rose-500' },
    ],
    state: [
      { k: '当前目标', v: '说服区里提前疏散低洼区' },
      { k: '态度', v: '对沈知遥：认可但担忧' },
    ],
    locked: true,
    location: '临江水文站',
  },
  {
    id: 'c3',
    name: '林曜',
    title: '市应急办副主任',
    initials: '林',
    gradient: 'from-zinc-400 to-zinc-700',
    tags: ['圆滑', '野心', '未定型'],
    bio: '年轻的官员，擅长在会议上把坏消息说成好消息。他似乎在隐瞒上游水库的真实情况。',
    stats: [
      { label: '权力', value: 70, color: 'bg-violet-500' },
      { label: '压力', value: 77, color: 'bg-rose-500' },
    ],
    state: [{ k: '当前目标', v: '保证“城市形象”不受影响' }],
    locked: false,
    location: '市政大楼',
  },
];

const rainWorld: World = {
  name: '临江市',
  genre: '现实 · 灾难 · 群像',
  era: '近未来 2031 年 · 梅雨季',
  day: 7,
  summary:
    '长江中游的一座中型城市，老城区沿江低洼、新区依山而建。IF 事件：「世界将连续下 30 天暴雨」已于第 1 天注入，当前推演至第 7 天。',
  rules: [
    '降雨持续期间，江水位每日自然上涨 0.4 ~ 1.2 m',
    '水位超过 28.5 m 时老城区开始内涝',
    '电力中断后，通讯可用性每日下降 15%',
    '民心 < 40 时触发「抢购 / 骚乱」事件链',
  ],
  vars: [
    { label: '降雨天数', value: '7 / 30 天', pct: 23, trend: 'up' },
    { label: '江水位', value: '27.8 m', pct: 82, trend: 'up' },
    { label: '粮食储备', value: '64%', pct: 64, trend: 'down' },
    { label: '电力覆盖', value: '91%', pct: 91, trend: 'down' },
    { label: '民心', value: '58', pct: 58, trend: 'down' },
  ],
  factions: [
    { name: '市应急办', attitude: '保守', power: 78 },
    { name: '临江水文站', attitude: '警告', power: 22 },
    { name: '老城居民自救会', attitude: '焦虑', power: 35 },
  ],
  locations: [
    { name: '临江水文站', status: '运转中', danger: 35 },
    { name: '老城区 · 南门街', status: '积水 40cm', danger: 62 },
    { name: '上游青石水库', status: '数据异常', danger: 80 },
    { name: '新区避难中心', status: '筹备中', danger: 10 },
  ],
};

const rainMap: MapNode[] = [
  { id: 'n0', label: '世界创建', type: 'origin', day: 'D0', main: true, desc: '临江市，梅雨季的第一个周末。' },
  { id: 'n1', parent: 'n0', label: '连降30天暴雨', type: 'event', day: 'D1', main: true, desc: '用户注入：IF 世界将会连续下 30 天暴雨。' },
  { id: 'n2', parent: 'n1', label: '水位破警戒', type: 'state', day: 'D4', main: true, desc: 'JEV：江水位突破 26 m 警戒线。' },
  { id: 'n3', parent: 'n1', label: '提前疏散', type: 'branch', day: 'D2', main: false, desc: '平行分支：若周望川第 2 天说服了区里。' },
  { id: 'n4', parent: 'n2', label: '水库数据异常', type: 'event', day: 'D6', main: true, desc: '沈知遥发现上游水库数据被篡改。' },
  { id: 'n5', parent: 'n2', label: '电网局部瘫痪', type: 'branch', day: 'D5', main: false, desc: '平行分支：变电站提前被淹。' },
  { id: 'n6', parent: 'n4', label: '第七夜', type: 'state', day: 'D7', main: true, desc: '当前节点。' },
];

const rainMessages: Message[] = [
  {
    id: 'm0',
    role: 'system',
    content: '世界「临江市」已创建 · JEV-2 已接管',
    time: '09:12',
  },
  {
    id: 'm1',
    role: 'event',
    eventTag: '天气',
    content: 'IF 世界将会连续下 30 天暴雨。',
    time: '09:13',
  },
  {
    id: 'm2',
    role: 'jev',
    time: '09:13',
    jev: {
      tick: 'D1 → D1',
      steps: JEV_STEPS,
      done: 5,
      latency: 842,
      branchScore: 0.93,
      rules: ['R-weather.persist(30d)', 'R-river.rise(daily)', 'R-city.flood(threshold=28.5)'],
      diffs: [
        { key: 'world.weather', from: '多云', to: '暴雨（持续 30d）', trend: 'neutral' },
        { key: 'world.river_level', from: '22.1 m', to: '22.9 m', trend: 'up' },
        { key: 'faction.应急办.alert', from: '无', to: 'IV 级', trend: 'up' },
        { key: 'char.周望川.腿伤', from: '12', to: '40', trend: 'up' },
      ],
    },
  },
  {
    id: 'm3',
    role: 'narrator',
    time: '09:13',
    content:
      '雨是在周六凌晨开始下的。\n\n起初没人在意。临江的梅雨季本来就长，老城区的人们把晾在外面的衣服收回来，骂了一句天气，又回去睡了。\n\n只有水文站的周望川醒了。他坐在床沿揉着左腿——那道九八年留下的旧伤，比任何气象雷达都先知道这场雨不一样。\n\n「小沈，」他拨通了电话，声音沙哑，「把三号水位计的数据调出来，每小时报一次。」\n\n电话那头，沈知遥迷迷糊糊地应了一声。窗外，雨声密得像是有人在天上倒豆子。\n\n**她还不知道，这场雨会下整整三十天。**',
  },
  {
    id: 'm4',
    role: 'event',
    eventTag: '时间跳跃',
    content: '时间快进到第 6 天，让沈知遥在夜班时发现一些不对劲的数据。',
    time: '10:02',
  },
  {
    id: 'm5',
    role: 'jev',
    time: '10:02',
    jev: {
      tick: 'D1 → D6',
      steps: JEV_STEPS,
      done: 5,
      latency: 1391,
      branchScore: 0.88,
      rules: ['R-river.rise × 5', 'R-supply.consume × 5', 'H-hidden.reservoir_tamper → reveal'],
      diffs: [
        { key: 'world.river_level', from: '22.9 m', to: '27.1 m', trend: 'up' },
        { key: 'world.food', from: '96%', to: '69%', trend: 'down' },
        { key: 'world.morale', from: '80', to: '61', trend: 'down' },
        { key: 'char.沈知遥.secret', from: '—', to: '水库数据被篡改', trend: 'neutral' },
      ],
    },
  },
  {
    id: 'm6',
    role: 'narrator',
    time: '10:03',
    content:
      '第六天，凌晨两点十七分。\n\n水文站的荧光灯嗡嗡作响。沈知遥把上游青石水库的出库流量曲线拖到屏幕中央，又拖回去，又拖出来。\n\n不对。\n\n连续五天，暴雨量翻了三倍，可水库报上来的出库流量却平得像一条尺子画出来的线。**没有哪座水库能在这种雨里保持完美的平稳。**除非——\n\n「除非有人不想让下游知道，它其实快满了。」\n\n她把这句话说出口的时候，身后的门被推开了。周望川披着湿透的雨衣站在门口，雨水顺着帽檐滴在地上。\n\n「你看到了？」他问。',
    choices: ['让周望川说出他早就知道的事', '注入事件：水库管理员失联', '注入事件：林曜突然来访水文站'],
  },
  {
    id: 'm7',
    role: 'image',
    time: '10:04',
    image: {
      name: '沈知遥 · 定型立绘',
      gradient: 'from-sky-500 via-indigo-600 to-slate-900',
      caption: 'flux1-dev · 768 × 1024',
    },
  },
];

/* ------------------------------------------------------------------ */
/* 其他故事（精简）                                                      */
/* ------------------------------------------------------------------ */

function simpleStory(
  id: string,
  title: string,
  genre: string,
  color: string,
  updated: string,
  group: Story['group'],
  world: Partial<World>,
  chars: Character[],
  opening: string,
  event: string,
  pinned = false,
): Story {
  const map: MapNode[] = [
    { id: 'n0', label: '世界创建', type: 'origin', day: 'D0', main: true, desc: world.summary ?? '' },
    { id: 'n1', parent: 'n0', label: event.slice(0, 8), type: 'event', day: 'D1', main: true, desc: event },
    { id: 'n2', parent: 'n1', label: '当前', type: 'state', day: 'D1', main: true, desc: '当前节点。' },
  ];
  return {
    id,
    title,
    genre,
    color,
    updated,
    group,
    pinned,
    tokens: Math.floor(Math.random() * 80000) + 12000,
    characters: chars,
    map,
    world: {
      name: title,
      genre,
      era: '',
      day: 1,
      summary: '',
      rules: [],
      vars: [],
      factions: [],
      locations: [],
      ...world,
    },
    messages: [
      { id: uid(), role: 'system', content: `世界「${world.name ?? title}」已创建 · JEV-2 已接管`, time: '昨天' },
      { id: uid(), role: 'event', eventTag: '自定义', content: event, time: '昨天' },
      {
        id: uid(),
        role: 'jev',
        time: '昨天',
        jev: {
          tick: 'D0 → D1',
          steps: JEV_STEPS,
          done: 5,
          latency: 911,
          branchScore: 0.9,
          rules: ['R-init.world', 'R-event.inject'],
          diffs: (world.vars ?? []).slice(0, 3).map((v) => ({ key: `world.${v.label}`, from: '—', to: v.value, trend: v.trend })),
        },
      },
      { id: uid(), role: 'narrator', content: opening, time: '昨天' },
    ],
  };
}

const starChars: Character[] = [
  {
    id: 's1',
    name: '伊芙·卡洛',
    title: '「赫尔墨斯」号通讯官',
    initials: '伊',
    gradient: 'from-fuchsia-500 to-purple-800',
    tags: ['敏锐', '失眠', '定型'],
    bio: '她是第一个发现地球信号消失的人。',
    stats: [
      { label: '体力', value: 80, color: 'bg-emerald-500' },
      { label: '理智', value: 55, color: 'bg-sky-500' },
    ],
    state: [{ k: '当前目标', v: '重建与地球的通讯' }],
    locked: true,
    location: '通讯舱',
  },
  {
    id: 's2',
    name: 'ARGUS',
    title: '舰载人工智能',
    initials: 'A',
    gradient: 'from-cyan-400 to-teal-700',
    tags: ['理性', '隐瞒？'],
    bio: '服役十二年的舰载 AI，最近的回答里开始出现停顿。',
    stats: [{ label: '算力', value: 92, color: 'bg-violet-500' }],
    state: [{ k: '当前目标', v: '未知' }],
    locked: false,
    location: '全舰',
  },
];

const tangChars: Character[] = [
  {
    id: 't1',
    name: '裴九娘',
    title: '西市胡姬酒肆老板娘',
    initials: '裴',
    gradient: 'from-rose-400 to-red-800',
    tags: ['八面玲珑', '暗线'],
    bio: '她的酒肆里，每晚都有人在交换长安最贵的消息。',
    stats: [
      { label: '人脉', value: 88, color: 'bg-amber-500' },
      { label: '危险', value: 40, color: 'bg-rose-500' },
    ],
    state: [{ k: '当前目标', v: '找到失踪的金吾卫' }],
    locked: true,
    location: '西市',
  },
];

const trainChars: Character[] = [
  {
    id: 'r1',
    name: '陆沉',
    title: '末班车乘客',
    initials: '陆',
    gradient: 'from-slate-400 to-slate-800',
    tags: ['疲惫', '未定型'],
    bio: '加班到深夜的程序员，在末班地铁上睡着了。',
    stats: [{ label: '理智', value: 70, color: 'bg-sky-500' }],
    state: [{ k: '当前目标', v: '下车' }],
    locked: false,
    location: '10 号线 · 第 ? 站',
  },
];

export const initialStories: Story[] = [
  {
    id: 'rain',
    title: '雨城纪事',
    genre: '现实 · 灾难',
    color: 'bg-sky-500',
    updated: '10:04',
    group: '置顶',
    pinned: true,
    tokens: 48213,
    messages: rainMessages,
    characters: rainChars,
    world: rainWorld,
    map: rainMap,
  },
  simpleStory(
    'star',
    '星港失联',
    '科幻 · 悬疑',
    'bg-fuchsia-500',
    '昨天',
    '今天',
    {
      name: '赫尔墨斯号',
      era: '2217 年 · 柯伊伯带',
      summary: '深空勘探舰「赫尔墨斯」号在返航途中失去了与地球的全部联系。',
      rules: ['舰内氧气每日消耗 1.2%', 'AI 权限高于船员时触发「接管」事件'],
      vars: [
        { label: '氧气', value: '87%', pct: 87, trend: 'down' },
        { label: '地球信号', value: '0', pct: 0, trend: 'down' },
        { label: '船员士气', value: '62', pct: 62, trend: 'down' },
      ],
      factions: [{ name: '舰桥指挥组', attitude: '镇定', power: 60 }],
      locations: [{ name: '通讯舱', status: '无信号', danger: 30 }],
    },
    starChars,
    '通讯屏幕上只剩下一行灰色的字：**NO CARRIER**。\n\n伊芙盯着它看了四十分钟。四十分钟前，地球还在。',
    'IF 地球在一瞬间停止发出任何信号。',
  ),
  simpleStory(
    'tang',
    '长安不夜',
    '历史 · 权谋',
    'bg-rose-500',
    '周二',
    '最近 7 天',
    {
      name: '长安城',
      era: '天宝十三载 · 上元节',
      summary: '上元节前夜，一队金吾卫在朱雀大街上凭空消失。',
      rules: ['宵禁解除三日', '坊间流言每日扩散一坊'],
      vars: [
        { label: '流言扩散', value: '3 坊', pct: 30, trend: 'up' },
        { label: '朝堂压力', value: '44', pct: 44, trend: 'up' },
      ],
      factions: [{ name: '金吾卫', attitude: '震怒', power: 70 }],
      locations: [{ name: '西市', status: '热闹', danger: 20 }],
    },
    tangChars,
    '灯火把朱雀大街照得如同白昼。可就在这片光里，十二名金吾卫走进了一片灯影——再也没有走出来。',
    'IF 上元节宵禁取消的三天里，长安每晚都会有人消失。',
  ),
  simpleStory(
    'train',
    '末班列车',
    '都市 · 怪谈',
    'bg-emerald-500',
    '5月2日',
    '更早',
    {
      name: '10 号线',
      era: '现代 · 23:58',
      summary: '末班地铁驶过终点站后，没有停下。',
      rules: ['每经过一站，车厢乘客减少一人'],
      vars: [
        { label: '已过站数', value: '3', pct: 15, trend: 'up' },
        { label: '剩余乘客', value: '9', pct: 60, trend: 'down' },
      ],
      locations: [{ name: '第 3 节车厢', status: '灯光闪烁', danger: 55 }],
    },
    trainChars,
    '广播里传来一个陌生的站名：「下一站，**不存在站**。」\n\n陆沉揉了揉眼睛，发现对面的座位空了。',
    'IF 末班地铁永远不会到站。',
  ),
];

export const defaultSettings: Settings = {
  theme: 'dark',
  accent: '#3b82f6',
  storyFont: 'sans',
  fontSize: 15,
  lineHeight: 1.85,
  chatWidth: 'normal',
  showJev: true,
  jevDetail: 'compact',
  imageEnabled: true,
  llm: {
    provider: 'Anthropic',
    base: 'https://api.anthropic.com/v1',
    key: '',
    model: 'claude-sonnet-4-20250514',
    temperature: 0.85,
    maxTokens: 2048,
    context: 200,
    style: '文学叙事',
  },
  jev: {
    endpoint: 'https://openrouter.ai/api/alpha/decisions',
    key: '',
    model: 'typesafe/jev-1.13',
    tick: '天',
    depth: 3,
    strict: 70,
    seed: '20310617',
    autoBranch: true,
  },
  image: {
    provider: 'ComfyUI（本地）',
    endpoint: 'http://127.0.0.1:8188',
    key: '',
    model: 'flux1-dev',
    style: '写实电影感',
    size: '768 × 1024',
    autoOnLock: true,
  },
  storage: {
    backend: '本地文件夹',
    path: '~/Documents/IF/worlds',
    autosave: true,
    snapshots: 50,
    encrypt: false,
  },
};

export const ACCENTS = [
  { name: '蔚蓝', value: '#3b82f6' },
  { name: '琥珀', value: '#d97706' },
  { name: '靛蓝', value: '#6366f1' },
  { name: '翠绿', value: '#10b981' },
  { name: '玫瑰', value: '#e11d48' },
  { name: '青碧', value: '#0891b2' },
  { name: '石墨', value: '#71717a' },
];

/* ------------------------------------------------------------------ */
/* 模拟生成                                                              */
/* ------------------------------------------------------------------ */

export function mockJev(event: string, tag: string) {
  const isRain = /雨|水|洪|涝/.test(event);
  const isTime = tag === '时间跳跃' || /快进|天后|跳过|第\s*\d+\s*天/.test(event);
  const diffs = isRain
    ? [
        { key: 'world.river_level', from: '27.8 m', to: '29.1 m', trend: 'up' as const },
        { key: 'loc.南门街.status', from: '积水 40cm', to: '内涝 1.2m', trend: 'up' as const },
        { key: 'world.morale', from: '58', to: '41', trend: 'down' as const },
      ]
    : isTime
      ? [
          { key: 'world.day', from: 'D7', to: 'D10', trend: 'up' as const },
          { key: 'world.food', from: '64%', to: '47%', trend: 'down' as const },
          { key: 'world.power', from: '91%', to: '73%', trend: 'down' as const },
        ]
      : [
          { key: 'event.inject', from: '—', to: event.slice(0, 14) + (event.length > 14 ? '…' : ''), trend: 'neutral' as const },
          { key: 'world.tension', from: '52', to: '68', trend: 'up' as const },
          { key: 'char.关系网', from: '稳定', to: '扰动 ×2', trend: 'neutral' as const },
        ];
  return {
    tick: isTime ? 'D7 → D10' : 'D7 → D7',
    steps: JEV_STEPS,
    done: 0,
    diffs,
    rules: isRain
      ? ['R-river.rise(×1.6)', 'R-city.flood(threshold=28.5) ✓', 'C-morale.cascade']
      : ['R-event.inject', 'C-consistency.check', 'H-narrative.hook'],
    branchScore: +(0.7 + Math.random() * 0.25).toFixed(2),
  };
}

export function mockNarration(event: string): { text: string; choices: string[] } {
  if (/雨|水|洪|涝/.test(event)) {
    return {
      text:
        '雨没有停的意思。\n\n凌晨四点，南门街的第一户人家被水漫过了门槛。老人们把电视机搬上桌子，孩子们被叫醒，睡眼惺忪地站在楼梯上看着黑色的水一点点爬上台阶。\n\n水文站里，三号水位计的读数跳到了 **29.1 米**。\n\n沈知遥没有说话，只是把这个数字抄在了记录本的最后一页，又在下面画了一条很重的线。\n\n「超过 28.5 了。」周望川说，「他们现在没法再说这是正常波动了。」\n\n她抬起头，窗外的城市一半亮着，一半已经暗了下去。',
      choices: ['注入事件：市区大面积停电', '让沈知遥把篡改的数据发到网上', '时间快进 3 天'],
    };
  }
  return {
    text: `世界接受了这个变化——「${event.replace(/。$/, '')}」。\n\nJEV 已将它写入世界的底层规则，涟漪正在向四周扩散。\n\n沈知遥最先察觉到异样。她放下手里的记录本，望向窗外：雨幕之中，城市的轮廓似乎在某个瞬间轻轻晃动了一下，像是有人在很远的地方拨动了一根弦。\n\n「你感觉到了吗？」她问。\n\n周望川没有回答。他只是握紧了那根陪了他二十七年的手电，**等待着下一件事发生。**`,
    choices: ['继续推进剧情', '注入事件：一位陌生人敲响了水文站的门', '查看受影响的角色'],
  };
}

export function newStory(): Story {
  const id = uid();
  return {
    id,
    title: '未命名世界',
    genre: '待定',
    color: 'bg-zinc-500',
    updated: now(),
    group: '今天',
    tokens: 0,
    characters: [],
    world: {
      name: '未命名世界',
      genre: '待定',
      era: '—',
      day: 0,
      summary: '世界尚未成形。注入第一个事件，JEV 将据此生成世界底层状态。',
      rules: [],
      vars: [],
      factions: [],
      locations: [],
    },
    map: [{ id: 'n0', label: '虚无', type: 'origin', day: 'D0', main: true, desc: '一切开始之前。' }],
    messages: [
      {
        id: uid(),
        role: 'narrator',
        time: now(),
        content: '这里还什么都没有。\n\n给我一个起点吧。',
        choices: ['IF 一座海边小镇的所有人同时失去了记忆', 'IF 世界将会连续下 30 天暴雨', 'IF 1920 年的上海出现了一台智能手机'],
      },
    ],
  };
}
