import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ComposedChart,
  LabelList,
  Line,
  Pie,
  PieChart,
  PolarAngleAxis,
  PolarGrid,
  PolarRadiusAxis,
  Radar,
  RadarChart,
  Rectangle,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

/*
 * The one module that imports recharts' components. Charts never import
 * "recharts" themselves (types excepted): they read this kit through
 * useRecharts(), which loads it with a single dynamic import.
 *
 * Why: the bundler gives every route its own copy of a library chunk when the
 * routes reach the library in a different order, so the dashboard, host,
 * monitor and report pages each shipped recharts (about 400 KB) again. Behind
 * one dynamic import there is exactly one recharts chunk, shared by every
 * page, and it stays out of the pages' first load. The object below names
 * every component the app draws with, so they all load together — add a
 * component here before using it in a chart.
 */
export const recharts = {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ComposedChart,
  LabelList,
  Line,
  Pie,
  PieChart,
  PolarAngleAxis,
  PolarGrid,
  PolarRadiusAxis,
  Radar,
  RadarChart,
  Rectangle,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
};

export type Recharts = typeof recharts;
