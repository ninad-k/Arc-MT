//+------------------------------------------------------------------+
//| MarketData.mqh - get_rates and get_calendar.                     |
//|                                                                  |
//| get_rates    {symbol,timeframe,count?} -> {symbol,timeframe,     |
//|                                  digits,bars[]}  oldest first    |
//| get_calendar {from,to,currencies?,min_importance?}               |
//|                                  -> {events[]}  sorted by time   |
//|                                                                  |
//| Both only read terminal state. They never trade.                 |
//| All times are server time (the same clock as bar times and       |
//| TimeTradeServer()), as MT5 reports them.                         |
//| Wire format: crates/bridge-client/src/protocol.rs                |
//+------------------------------------------------------------------+
#ifndef JARVIS_MARKET_DATA_MQH
#define JARVIS_MARKET_DATA_MQH

#include "Json.mqh"
#include "ReadHandlers.mqh"

#define JARVIS_RATES_COUNT_DEFAULT   300
#define JARVIS_RATES_COUNT_MAX       1000
#define JARVIS_RC_HISTORY_NOT_FOUND  4401   // ERR_HISTORY_NOT_FOUND

#define JARVIS_CALENDAR_RANGE_MAX    1209600   // 14 days in seconds
#define JARVIS_CALENDAR_EVENTS_MAX   500
#define JARVIS_CALENDAR_CURRENCIES_MAX 16

//--- "M15" -> PERIOD_M15. Exact, upper-case names only (the wire is exact).
bool Jarvis_ParseTimeframe(const string name,ENUM_TIMEFRAMES &tf)
  {
   if(name=="M1")
     {
      tf=PERIOD_M1;
      return true;
     }
   if(name=="M5")
     {
      tf=PERIOD_M5;
      return true;
     }
   if(name=="M15")
     {
      tf=PERIOD_M15;
      return true;
     }
   if(name=="M30")
     {
      tf=PERIOD_M30;
      return true;
     }
   if(name=="H1")
     {
      tf=PERIOD_H1;
      return true;
     }
   if(name=="H4")
     {
      tf=PERIOD_H4;
      return true;
     }
   if(name=="D1")
     {
      tf=PERIOD_D1;
      return true;
     }
   return false;
  }

//+------------------------------------------------------------------+
//| get_rates {symbol,timeframe,count?}                              |
//| count: 1..1000 (default 300). The last bar is the one still      |
//| forming. Fewer bars than asked for is not an error. If the       |
//| terminal has no bars at all yet (history still loading) the      |
//| answer is the MT5 error (4401 when MT5 reports none): the client |
//| asks again after a short pause. A reply of 1000 bars is about    |
//| 120 KB of JSON, well inside what the socket sends (a             |
//| list_symbols page is up to 250 KB); nothing is cut off.          |
//+------------------------------------------------------------------+
int Jarvis_CmdGetRates(CJson &req,const int args,CJsonWriter &w,string &err)
  {
   string sym,tfName;
   bool hasSym,hasTf,hasCount;
   long count=0;
   if(!Jarvis_ArgString(req,args,"symbol",sym,hasSym,err))
      return JARVIS_RC_INVALID;
   if(!Jarvis_ArgString(req,args,"timeframe",tfName,hasTf,err))
      return JARVIS_RC_INVALID;
   if(!Jarvis_ArgInt(req,args,"count",count,hasCount,err))
      return JARVIS_RC_INVALID;
   if(!hasSym)
      return Jarvis_Invalid(err,"missing argument 'symbol'");
   if(!hasTf)
      return Jarvis_Invalid(err,"missing argument 'timeframe'");
   ENUM_TIMEFRAMES tf=PERIOD_CURRENT;
   if(!Jarvis_ParseTimeframe(tfName,tf))
      return Jarvis_Invalid(err,"'timeframe' must be one of M1, M5, M15, M30, H1, H4, D1");
   if(!hasCount)
      count=JARVIS_RATES_COUNT_DEFAULT;
   if(count<1 || count>JARVIS_RATES_COUNT_MAX)
      return Jarvis_Invalid(err,StringFormat("'count' must be between 1 and %d",JARVIS_RATES_COUNT_MAX));
   int rc=0;
   if(!Jarvis_CheckSymbol(sym,true,rc,err))
      return rc;

   MqlRates rates[];
   ArraySetAsSeries(rates,false);   // oldest bar first
   ResetLastError();
   int copied=CopyRates(sym,tf,0,(int)count,rates);
   if(copied<=0)
     {
      int e=(copied<0)?GetLastError():0;
      if(e==0)
         e=JARVIS_RC_HISTORY_NOT_FOUND;
      err="no bars for "+sym+" "+tfName+" (history may still be loading)";
      return e;
     }
   int digits=Jarvis_Digits(sym);

   w.BeginObject();
   w.PutStr("symbol",sym);
   w.PutStr("timeframe",tfName);
   w.PutInt("digits",(digits<0)?0:digits);
   w.Key("bars");
   w.BeginArray();
   for(int i=0; i<copied; i++)
     {
      w.BeginObject();
      w.PutInt("time",(long)rates[i].time);
      w.PutNum("open",rates[i].open,digits);
      w.PutNum("high",rates[i].high,digits);
      w.PutNum("low",rates[i].low,digits);
      w.PutNum("close",rates[i].close,digits);
      w.PutUInt("tick_volume",(ulong)MathMax(0,rates[i].tick_volume));
      w.PutInt("spread",(long)rates[i].spread);
      w.EndObject();
     }
   w.EndArray();
   w.EndObject();
   return 0;
  }

//--- CALENDAR_IMPORTANCE_* -> 0 low (also NONE), 1 moderate, 2 high
int Jarvis_ImportanceRank(const ENUM_CALENDAR_EVENT_IMPORTANCE imp)
  {
   if(imp==CALENDAR_IMPORTANCE_HIGH)
      return 2;
   if(imp==CALENDAR_IMPORTANCE_MODERATE)
      return 1;
   return 0;
  }

string Jarvis_ImportanceName(const int rank)
  {
   if(rank>=2)
      return "high";
   if(rank==1)
      return "moderate";
   return "low";
  }

//--- display text of the unit of an event's values, e.g. "%", "K", "$B", "K jobs".
//--- Empty when the event has neither a unit nor a multiplier.
string Jarvis_CalendarUnit(const MqlCalendarEvent &ev,const MqlCalendarCountry &country)
  {
   string mult="";
   switch(ev.multiplier)
     {
      case CALENDAR_MULTIPLIER_THOUSANDS:
         mult="K";
         break;
      case CALENDAR_MULTIPLIER_MILLIONS:
         mult="M";
         break;
      case CALENDAR_MULTIPLIER_BILLIONS:
         mult="B";
         break;
      case CALENDAR_MULTIPLIER_TRILLIONS:
         mult="T";
         break;
      default:
         break;
     }
   string sym="";    // a symbol that sits right before the multiplier ("$B")
   string word="";   // a word that follows it ("K jobs")
   switch(ev.unit)
     {
      case CALENDAR_UNIT_PERCENT:
         sym="%";
         break;
      case CALENDAR_UNIT_CURRENCY:
         sym=country.currency_symbol;
         break;
      case CALENDAR_UNIT_USD:
         sym="$";
         break;
      case CALENDAR_UNIT_HOUR:
         word="h";
         break;
      case CALENDAR_UNIT_JOB:
         word="jobs";
         break;
      case CALENDAR_UNIT_RIG:
         word="rigs";
         break;
      case CALENDAR_UNIT_PEOPLE:
         word="people";
         break;
      case CALENDAR_UNIT_MORTGAGE:
         word="mortgages";
         break;
      case CALENDAR_UNIT_VOTE:
         word="votes";
         break;
      case CALENDAR_UNIT_BARREL:
         word="bbl";
         break;
      case CALENDAR_UNIT_CUBICFEET:
         word="cu ft";
         break;
      case CALENDAR_UNIT_POSITION:
         word="positions";
         break;
      case CALENDAR_UNIT_BUILDING:
         word="buildings";
         break;
      default:
         break;
     }
   string text=sym+mult;
   if(word!="")
      text+=(text=="")?word:" "+word;
   return text;
  }

//--- stable bottom-up merge sort of order[0..n-1] by keys[order[i]] ascending
bool Jarvis_SortIndexByKey(const long &keys[],int &order[],const int n)
  {
   if(n<2)
      return true;
   int dst[];
   if(ArrayResize(dst,n)!=n)
      return false;
   for(int width=1; width<n; width*=2)
     {
      for(int lo=0; lo<n; lo+=2*width)
        {
         int mid=MathMin(lo+width,n);
         int hi=MathMin(lo+2*width,n);
         int a=lo,b=mid,k=lo;
         while(a<mid && b<hi)
           {
            if(keys[order[b]]<keys[order[a]])
               dst[k++]=order[b++];
            else
               dst[k++]=order[a++];
           }
         while(a<mid)
            dst[k++]=order[a++];
         while(b<hi)
            dst[k++]=order[b++];
        }
      ArrayCopy(order,dst,0,0,n);
     }
   return true;
  }

//--- optional "currencies": array of 3-letter codes, upper-cased and de-duplicated.
//--- false => err set.
bool Jarvis_ArgCurrencies(CJson &j,const int args,string &list[],int &count,string &err)
  {
   count=0;
   int node=j.Get(args,"currencies");
   if(node<0 || j.Type(node)==JV_NULL)
      return true;
   if(j.Type(node)!=JV_ARRAY)
     {
      err="argument 'currencies' must be an array of currency codes";
      return false;
     }
   int n=j.Count(node);
   if(n>JARVIS_CALENDAR_CURRENCIES_MAX)
     {
      err=StringFormat("argument 'currencies' has more than %d entries",JARVIS_CALENDAR_CURRENCIES_MAX);
      return false;
     }
   if(n>0 && ArrayResize(list,n)!=n)
     {
      err="out of memory";
      return false;
     }
   for(int i=0; i<n; i++)
     {
      int item=j.At(node,i);
      string code=j.Str(item);
      bool valid=j.IsString(item) && StringLen(code)==3;
      for(int c=0; valid && c<3; c++)
        {
         ushort ch=StringGetCharacter(code,c);
         valid=(ch>='A' && ch<='Z') || (ch>='a' && ch<='z');
        }
      if(!valid)
        {
         err="argument 'currencies' must contain three-letter currency codes such as \"USD\"";
         return false;
        }
      StringToUpper(code);
      bool seen=false;
      for(int k=0; k<count && !seen; k++)
         seen=(list[k]==code);
      if(!seen)
         list[count++]=code;
     }
   return true;
  }

//+------------------------------------------------------------------+
//| get_calendar {from,to,currencies?,min_importance?}               |
//| from, to: server-time Unix seconds, to after from, at most 14    |
//| days apart. currencies: empty or missing = all. min_importance:  |
//| "low", "moderate" (default) or "high"; events MT5 rates "none"   |
//| count as low. Events are sorted by time; at most 500 are         |
//| returned (the earliest). actual/forecast/previous are null when  |
//| MT5 has no value; "previous" is the revised previous value when  |
//| one exists. "event" is in the terminal's language.               |
//+------------------------------------------------------------------+
int Jarvis_CmdGetCalendar(CJson &req,const int args,CJsonWriter &w,string &err)
  {
   long from=0,to=0;
   bool hasFrom,hasTo,hasMin;
   string minName;
   if(!Jarvis_ArgInt(req,args,"from",from,hasFrom,err))
      return JARVIS_RC_INVALID;
   if(!Jarvis_ArgInt(req,args,"to",to,hasTo,err))
      return JARVIS_RC_INVALID;
   if(!Jarvis_ArgString(req,args,"min_importance",minName,hasMin,err))
      return JARVIS_RC_INVALID;
   string currencies[];
   int currencyCount=0;
   if(!Jarvis_ArgCurrencies(req,args,currencies,currencyCount,err))
      return JARVIS_RC_INVALID;
   if(!hasFrom)
      return Jarvis_Invalid(err,"missing argument 'from'");
   if(!hasTo)
      return Jarvis_Invalid(err,"missing argument 'to'");
   if(from<0 || to<0 || from>JARVIS_MAX_DATETIME || to>JARVIS_MAX_DATETIME)
      return Jarvis_Invalid(err,"'from' and 'to' must be Unix seconds in a valid range");
   if(to<=from)
      return Jarvis_Invalid(err,"'to' must be after 'from'");
   if(to-from>JARVIS_CALENDAR_RANGE_MAX)
      return Jarvis_Invalid(err,"the range between 'from' and 'to' must be at most 14 days");
   int minRank=1;
   if(hasMin)
     {
      if(minName=="low")
         minRank=0;
      else
         if(minName=="moderate")
            minRank=1;
         else
            if(minName=="high")
               minRank=2;
            else
               return Jarvis_Invalid(err,"'min_importance' must be \"low\", \"moderate\" or \"high\"");
     }

   //--- all values of the range, one request per currency (or one for everything)
   MqlCalendarValue values[];
   int total=0;
   int requests=(currencyCount>0)?currencyCount:1;
   for(int r=0; r<requests; r++)
     {
      MqlCalendarValue part[];
      ResetLastError();
      int got;
      if(currencyCount>0)
         got=CalendarValueHistory(part,(datetime)from,(datetime)to,NULL,currencies[r]);
      else
         got=CalendarValueHistory(part,(datetime)from,(datetime)to);
      if(got<0)
         return Jarvis_MtFail(err,"the economic calendar is not available");
      if(got==0)
         continue;
      if(ArrayResize(values,total+got)!=total+got)
        {
         err="out of memory while reading the economic calendar";
         return JARVIS_RC_INTERNAL;
        }
      ArrayCopy(values,part,total,0,got);
      total+=got;
     }

   //--- keep the values that pass the importance filter
   int keep[];
   long keys[];
   if(total>0 && (ArrayResize(keep,total)!=total || ArrayResize(keys,total)!=total))
     {
      err="out of memory while reading the economic calendar";
      return JARVIS_RC_INTERNAL;
     }
   int kept=0;
   for(int i=0; i<total; i++)
     {
      // CalendarValueHistory can return values from before 'from' (seen live), so
      // apply the range again here.
      if((long)values[i].time<from || (long)values[i].time>to)
         continue;
      MqlCalendarEvent ev;
      if(!CalendarEventById(values[i].event_id,ev))
         continue;   // the event vanished from the calendar meanwhile
      if(Jarvis_ImportanceRank(ev.importance)<minRank)
         continue;
      keys[i]=(long)values[i].time;
      keep[kept++]=i;
     }
   if(!Jarvis_SortIndexByKey(keys,keep,kept))
     {
      err="out of memory while sorting the economic calendar";
      return JARVIS_RC_INTERNAL;
     }

   w.BeginObject();
   w.Key("events");
   w.BeginArray();
   int written=0;
   for(int i=0; i<kept && written<JARVIS_CALENDAR_EVENTS_MAX; i++)
     {
      int idx=keep[i];
      MqlCalendarEvent ev;
      MqlCalendarCountry country;
      if(!CalendarEventById(values[idx].event_id,ev))
         continue;
      if(!CalendarCountryById((long)ev.country_id,country))
         continue;
      w.BeginObject();
      w.PutInt("time",(long)values[idx].time);
      w.PutStr("currency",country.currency);
      w.PutStr("country",country.code);
      w.PutStr("event",ev.name);
      w.PutStr("importance",Jarvis_ImportanceName(Jarvis_ImportanceRank(ev.importance)));
      if(values[idx].HasActualValue())
         w.PutNum("actual",values[idx].GetActualValue());
      else
         w.PutNull("actual");
      if(values[idx].HasForecastValue())
         w.PutNum("forecast",values[idx].GetForecastValue());
      else
         w.PutNull("forecast");
      if(values[idx].HasRevisedValue())
         w.PutNum("previous",values[idx].GetRevisedValue());
      else
         if(values[idx].HasPreviousValue())
            w.PutNum("previous",values[idx].GetPreviousValue());
         else
            w.PutNull("previous");
      w.PutStr("unit",Jarvis_CalendarUnit(ev,country));
      w.EndObject();
      written++;
     }
   w.EndArray();
   w.EndObject();
   return 0;
  }

#endif // JARVIS_MARKET_DATA_MQH
