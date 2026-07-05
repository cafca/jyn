// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'runtime.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$JynEvent {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'JynEvent()';
}


}

/// @nodoc
class $JynEventCopyWith<$Res>  {
$JynEventCopyWith(JynEvent _, $Res Function(JynEvent) __);
}


/// Adds pattern-matching-related methods to [JynEvent].
extension JynEventPatterns on JynEvent {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( JynEvent_River value)?  river,TResult Function( JynEvent_Profile value)?  profile,TResult Function( JynEvent_Friends value)?  friends,TResult Function( JynEvent_Diagnostics value)?  diagnostics,TResult Function( JynEvent_MediaReady value)?  mediaReady,TResult Function( JynEvent_MediaFailed value)?  mediaFailed,TResult Function( JynEvent_Error value)?  error,required TResult orElse(),}){
final _that = this;
switch (_that) {
case JynEvent_River() when river != null:
return river(_that);case JynEvent_Profile() when profile != null:
return profile(_that);case JynEvent_Friends() when friends != null:
return friends(_that);case JynEvent_Diagnostics() when diagnostics != null:
return diagnostics(_that);case JynEvent_MediaReady() when mediaReady != null:
return mediaReady(_that);case JynEvent_MediaFailed() when mediaFailed != null:
return mediaFailed(_that);case JynEvent_Error() when error != null:
return error(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( JynEvent_River value)  river,required TResult Function( JynEvent_Profile value)  profile,required TResult Function( JynEvent_Friends value)  friends,required TResult Function( JynEvent_Diagnostics value)  diagnostics,required TResult Function( JynEvent_MediaReady value)  mediaReady,required TResult Function( JynEvent_MediaFailed value)  mediaFailed,required TResult Function( JynEvent_Error value)  error,}){
final _that = this;
switch (_that) {
case JynEvent_River():
return river(_that);case JynEvent_Profile():
return profile(_that);case JynEvent_Friends():
return friends(_that);case JynEvent_Diagnostics():
return diagnostics(_that);case JynEvent_MediaReady():
return mediaReady(_that);case JynEvent_MediaFailed():
return mediaFailed(_that);case JynEvent_Error():
return error(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( JynEvent_River value)?  river,TResult? Function( JynEvent_Profile value)?  profile,TResult? Function( JynEvent_Friends value)?  friends,TResult? Function( JynEvent_Diagnostics value)?  diagnostics,TResult? Function( JynEvent_MediaReady value)?  mediaReady,TResult? Function( JynEvent_MediaFailed value)?  mediaFailed,TResult? Function( JynEvent_Error value)?  error,}){
final _that = this;
switch (_that) {
case JynEvent_River() when river != null:
return river(_that);case JynEvent_Profile() when profile != null:
return profile(_that);case JynEvent_Friends() when friends != null:
return friends(_that);case JynEvent_Diagnostics() when diagnostics != null:
return diagnostics(_that);case JynEvent_MediaReady() when mediaReady != null:
return mediaReady(_that);case JynEvent_MediaFailed() when mediaFailed != null:
return mediaFailed(_that);case JynEvent_Error() when error != null:
return error(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( List<RiverPost> posts,  List<GhostCard> ghosts)?  river,TResult Function( UserProfile profile)?  profile,TResult Function( List<FriendEntry> friends,  List<PendingFriendRequest> pending)?  friends,TResult Function( DiagnosticsSnapshot snapshot)?  diagnostics,TResult Function( String blobHash,  String path)?  mediaReady,TResult Function( String blobHash,  String errorMessage)?  mediaFailed,TResult Function( String context,  String message)?  error,required TResult orElse(),}) {final _that = this;
switch (_that) {
case JynEvent_River() when river != null:
return river(_that.posts,_that.ghosts);case JynEvent_Profile() when profile != null:
return profile(_that.profile);case JynEvent_Friends() when friends != null:
return friends(_that.friends,_that.pending);case JynEvent_Diagnostics() when diagnostics != null:
return diagnostics(_that.snapshot);case JynEvent_MediaReady() when mediaReady != null:
return mediaReady(_that.blobHash,_that.path);case JynEvent_MediaFailed() when mediaFailed != null:
return mediaFailed(_that.blobHash,_that.errorMessage);case JynEvent_Error() when error != null:
return error(_that.context,_that.message);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( List<RiverPost> posts,  List<GhostCard> ghosts)  river,required TResult Function( UserProfile profile)  profile,required TResult Function( List<FriendEntry> friends,  List<PendingFriendRequest> pending)  friends,required TResult Function( DiagnosticsSnapshot snapshot)  diagnostics,required TResult Function( String blobHash,  String path)  mediaReady,required TResult Function( String blobHash,  String errorMessage)  mediaFailed,required TResult Function( String context,  String message)  error,}) {final _that = this;
switch (_that) {
case JynEvent_River():
return river(_that.posts,_that.ghosts);case JynEvent_Profile():
return profile(_that.profile);case JynEvent_Friends():
return friends(_that.friends,_that.pending);case JynEvent_Diagnostics():
return diagnostics(_that.snapshot);case JynEvent_MediaReady():
return mediaReady(_that.blobHash,_that.path);case JynEvent_MediaFailed():
return mediaFailed(_that.blobHash,_that.errorMessage);case JynEvent_Error():
return error(_that.context,_that.message);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( List<RiverPost> posts,  List<GhostCard> ghosts)?  river,TResult? Function( UserProfile profile)?  profile,TResult? Function( List<FriendEntry> friends,  List<PendingFriendRequest> pending)?  friends,TResult? Function( DiagnosticsSnapshot snapshot)?  diagnostics,TResult? Function( String blobHash,  String path)?  mediaReady,TResult? Function( String blobHash,  String errorMessage)?  mediaFailed,TResult? Function( String context,  String message)?  error,}) {final _that = this;
switch (_that) {
case JynEvent_River() when river != null:
return river(_that.posts,_that.ghosts);case JynEvent_Profile() when profile != null:
return profile(_that.profile);case JynEvent_Friends() when friends != null:
return friends(_that.friends,_that.pending);case JynEvent_Diagnostics() when diagnostics != null:
return diagnostics(_that.snapshot);case JynEvent_MediaReady() when mediaReady != null:
return mediaReady(_that.blobHash,_that.path);case JynEvent_MediaFailed() when mediaFailed != null:
return mediaFailed(_that.blobHash,_that.errorMessage);case JynEvent_Error() when error != null:
return error(_that.context,_that.message);case _:
  return null;

}
}

}

/// @nodoc


class JynEvent_River extends JynEvent {
  const JynEvent_River({required final  List<RiverPost> posts, required final  List<GhostCard> ghosts}): _posts = posts,_ghosts = ghosts,super._();
  

 final  List<RiverPost> _posts;
 List<RiverPost> get posts {
  if (_posts is EqualUnmodifiableListView) return _posts;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_posts);
}

 final  List<GhostCard> _ghosts;
 List<GhostCard> get ghosts {
  if (_ghosts is EqualUnmodifiableListView) return _ghosts;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_ghosts);
}


/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_RiverCopyWith<JynEvent_River> get copyWith => _$JynEvent_RiverCopyWithImpl<JynEvent_River>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_River&&const DeepCollectionEquality().equals(other._posts, _posts)&&const DeepCollectionEquality().equals(other._ghosts, _ghosts));
}


@override
int get hashCode => Object.hash(runtimeType,const DeepCollectionEquality().hash(_posts),const DeepCollectionEquality().hash(_ghosts));

@override
String toString() {
  return 'JynEvent.river(posts: $posts, ghosts: $ghosts)';
}


}

/// @nodoc
abstract mixin class $JynEvent_RiverCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_RiverCopyWith(JynEvent_River value, $Res Function(JynEvent_River) _then) = _$JynEvent_RiverCopyWithImpl;
@useResult
$Res call({
 List<RiverPost> posts, List<GhostCard> ghosts
});




}
/// @nodoc
class _$JynEvent_RiverCopyWithImpl<$Res>
    implements $JynEvent_RiverCopyWith<$Res> {
  _$JynEvent_RiverCopyWithImpl(this._self, this._then);

  final JynEvent_River _self;
  final $Res Function(JynEvent_River) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? posts = null,Object? ghosts = null,}) {
  return _then(JynEvent_River(
posts: null == posts ? _self._posts : posts // ignore: cast_nullable_to_non_nullable
as List<RiverPost>,ghosts: null == ghosts ? _self._ghosts : ghosts // ignore: cast_nullable_to_non_nullable
as List<GhostCard>,
  ));
}


}

/// @nodoc


class JynEvent_Profile extends JynEvent {
  const JynEvent_Profile({required this.profile}): super._();
  

 final  UserProfile profile;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_ProfileCopyWith<JynEvent_Profile> get copyWith => _$JynEvent_ProfileCopyWithImpl<JynEvent_Profile>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_Profile&&(identical(other.profile, profile) || other.profile == profile));
}


@override
int get hashCode => Object.hash(runtimeType,profile);

@override
String toString() {
  return 'JynEvent.profile(profile: $profile)';
}


}

/// @nodoc
abstract mixin class $JynEvent_ProfileCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_ProfileCopyWith(JynEvent_Profile value, $Res Function(JynEvent_Profile) _then) = _$JynEvent_ProfileCopyWithImpl;
@useResult
$Res call({
 UserProfile profile
});




}
/// @nodoc
class _$JynEvent_ProfileCopyWithImpl<$Res>
    implements $JynEvent_ProfileCopyWith<$Res> {
  _$JynEvent_ProfileCopyWithImpl(this._self, this._then);

  final JynEvent_Profile _self;
  final $Res Function(JynEvent_Profile) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? profile = null,}) {
  return _then(JynEvent_Profile(
profile: null == profile ? _self.profile : profile // ignore: cast_nullable_to_non_nullable
as UserProfile,
  ));
}


}

/// @nodoc


class JynEvent_Friends extends JynEvent {
  const JynEvent_Friends({required final  List<FriendEntry> friends, required final  List<PendingFriendRequest> pending}): _friends = friends,_pending = pending,super._();
  

 final  List<FriendEntry> _friends;
 List<FriendEntry> get friends {
  if (_friends is EqualUnmodifiableListView) return _friends;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_friends);
}

 final  List<PendingFriendRequest> _pending;
 List<PendingFriendRequest> get pending {
  if (_pending is EqualUnmodifiableListView) return _pending;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_pending);
}


/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_FriendsCopyWith<JynEvent_Friends> get copyWith => _$JynEvent_FriendsCopyWithImpl<JynEvent_Friends>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_Friends&&const DeepCollectionEquality().equals(other._friends, _friends)&&const DeepCollectionEquality().equals(other._pending, _pending));
}


@override
int get hashCode => Object.hash(runtimeType,const DeepCollectionEquality().hash(_friends),const DeepCollectionEquality().hash(_pending));

@override
String toString() {
  return 'JynEvent.friends(friends: $friends, pending: $pending)';
}


}

/// @nodoc
abstract mixin class $JynEvent_FriendsCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_FriendsCopyWith(JynEvent_Friends value, $Res Function(JynEvent_Friends) _then) = _$JynEvent_FriendsCopyWithImpl;
@useResult
$Res call({
 List<FriendEntry> friends, List<PendingFriendRequest> pending
});




}
/// @nodoc
class _$JynEvent_FriendsCopyWithImpl<$Res>
    implements $JynEvent_FriendsCopyWith<$Res> {
  _$JynEvent_FriendsCopyWithImpl(this._self, this._then);

  final JynEvent_Friends _self;
  final $Res Function(JynEvent_Friends) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? friends = null,Object? pending = null,}) {
  return _then(JynEvent_Friends(
friends: null == friends ? _self._friends : friends // ignore: cast_nullable_to_non_nullable
as List<FriendEntry>,pending: null == pending ? _self._pending : pending // ignore: cast_nullable_to_non_nullable
as List<PendingFriendRequest>,
  ));
}


}

/// @nodoc


class JynEvent_Diagnostics extends JynEvent {
  const JynEvent_Diagnostics({required this.snapshot}): super._();
  

 final  DiagnosticsSnapshot snapshot;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_DiagnosticsCopyWith<JynEvent_Diagnostics> get copyWith => _$JynEvent_DiagnosticsCopyWithImpl<JynEvent_Diagnostics>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_Diagnostics&&(identical(other.snapshot, snapshot) || other.snapshot == snapshot));
}


@override
int get hashCode => Object.hash(runtimeType,snapshot);

@override
String toString() {
  return 'JynEvent.diagnostics(snapshot: $snapshot)';
}


}

/// @nodoc
abstract mixin class $JynEvent_DiagnosticsCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_DiagnosticsCopyWith(JynEvent_Diagnostics value, $Res Function(JynEvent_Diagnostics) _then) = _$JynEvent_DiagnosticsCopyWithImpl;
@useResult
$Res call({
 DiagnosticsSnapshot snapshot
});




}
/// @nodoc
class _$JynEvent_DiagnosticsCopyWithImpl<$Res>
    implements $JynEvent_DiagnosticsCopyWith<$Res> {
  _$JynEvent_DiagnosticsCopyWithImpl(this._self, this._then);

  final JynEvent_Diagnostics _self;
  final $Res Function(JynEvent_Diagnostics) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? snapshot = null,}) {
  return _then(JynEvent_Diagnostics(
snapshot: null == snapshot ? _self.snapshot : snapshot // ignore: cast_nullable_to_non_nullable
as DiagnosticsSnapshot,
  ));
}


}

/// @nodoc


class JynEvent_MediaReady extends JynEvent {
  const JynEvent_MediaReady({required this.blobHash, required this.path}): super._();
  

 final  String blobHash;
 final  String path;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_MediaReadyCopyWith<JynEvent_MediaReady> get copyWith => _$JynEvent_MediaReadyCopyWithImpl<JynEvent_MediaReady>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_MediaReady&&(identical(other.blobHash, blobHash) || other.blobHash == blobHash)&&(identical(other.path, path) || other.path == path));
}


@override
int get hashCode => Object.hash(runtimeType,blobHash,path);

@override
String toString() {
  return 'JynEvent.mediaReady(blobHash: $blobHash, path: $path)';
}


}

/// @nodoc
abstract mixin class $JynEvent_MediaReadyCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_MediaReadyCopyWith(JynEvent_MediaReady value, $Res Function(JynEvent_MediaReady) _then) = _$JynEvent_MediaReadyCopyWithImpl;
@useResult
$Res call({
 String blobHash, String path
});




}
/// @nodoc
class _$JynEvent_MediaReadyCopyWithImpl<$Res>
    implements $JynEvent_MediaReadyCopyWith<$Res> {
  _$JynEvent_MediaReadyCopyWithImpl(this._self, this._then);

  final JynEvent_MediaReady _self;
  final $Res Function(JynEvent_MediaReady) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? blobHash = null,Object? path = null,}) {
  return _then(JynEvent_MediaReady(
blobHash: null == blobHash ? _self.blobHash : blobHash // ignore: cast_nullable_to_non_nullable
as String,path: null == path ? _self.path : path // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class JynEvent_MediaFailed extends JynEvent {
  const JynEvent_MediaFailed({required this.blobHash, required this.errorMessage}): super._();
  

 final  String blobHash;
 final  String errorMessage;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_MediaFailedCopyWith<JynEvent_MediaFailed> get copyWith => _$JynEvent_MediaFailedCopyWithImpl<JynEvent_MediaFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_MediaFailed&&(identical(other.blobHash, blobHash) || other.blobHash == blobHash)&&(identical(other.errorMessage, errorMessage) || other.errorMessage == errorMessage));
}


@override
int get hashCode => Object.hash(runtimeType,blobHash,errorMessage);

@override
String toString() {
  return 'JynEvent.mediaFailed(blobHash: $blobHash, errorMessage: $errorMessage)';
}


}

/// @nodoc
abstract mixin class $JynEvent_MediaFailedCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_MediaFailedCopyWith(JynEvent_MediaFailed value, $Res Function(JynEvent_MediaFailed) _then) = _$JynEvent_MediaFailedCopyWithImpl;
@useResult
$Res call({
 String blobHash, String errorMessage
});




}
/// @nodoc
class _$JynEvent_MediaFailedCopyWithImpl<$Res>
    implements $JynEvent_MediaFailedCopyWith<$Res> {
  _$JynEvent_MediaFailedCopyWithImpl(this._self, this._then);

  final JynEvent_MediaFailed _self;
  final $Res Function(JynEvent_MediaFailed) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? blobHash = null,Object? errorMessage = null,}) {
  return _then(JynEvent_MediaFailed(
blobHash: null == blobHash ? _self.blobHash : blobHash // ignore: cast_nullable_to_non_nullable
as String,errorMessage: null == errorMessage ? _self.errorMessage : errorMessage // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class JynEvent_Error extends JynEvent {
  const JynEvent_Error({required this.context, required this.message}): super._();
  

 final  String context;
 final  String message;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JynEvent_ErrorCopyWith<JynEvent_Error> get copyWith => _$JynEvent_ErrorCopyWithImpl<JynEvent_Error>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is JynEvent_Error&&(identical(other.context, context) || other.context == context)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,context,message);

@override
String toString() {
  return 'JynEvent.error(context: $context, message: $message)';
}


}

/// @nodoc
abstract mixin class $JynEvent_ErrorCopyWith<$Res> implements $JynEventCopyWith<$Res> {
  factory $JynEvent_ErrorCopyWith(JynEvent_Error value, $Res Function(JynEvent_Error) _then) = _$JynEvent_ErrorCopyWithImpl;
@useResult
$Res call({
 String context, String message
});




}
/// @nodoc
class _$JynEvent_ErrorCopyWithImpl<$Res>
    implements $JynEvent_ErrorCopyWith<$Res> {
  _$JynEvent_ErrorCopyWithImpl(this._self, this._then);

  final JynEvent_Error _self;
  final $Res Function(JynEvent_Error) _then;

/// Create a copy of JynEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? context = null,Object? message = null,}) {
  return _then(JynEvent_Error(
context: null == context ? _self.context : context // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

// dart format on
